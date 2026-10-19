//! Serial execution and ordered parallel batch merging.
use crate::engine::{self, AutoNames, GenOpts, Scratch};
use crate::fmt::{Ctx, Plugin};
use crate::names::Person;
use crate::output::{self, Emitter};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Where the people to generate come from.
///
/// `Auto` deliberately holds the two dictionaries rather than a materialized
/// cross-product: `-a` with the Facebook lists is 10^8 combinations, and
/// building them up front would dominate the run.
#[derive(Clone, Copy)]
pub enum Work<'a> {
    Fixed(&'a [Person<'a>]),
    Auto {
        head: &'a [Person<'a>],
        names: AutoNames<'a>,
    },
}

fn output_hint(work: Work<'_>, est: u64) -> usize {
    // A cross-product can emit millions of copies of a few distinct names.
    // Reserve a small table for auto mode and let it grow with unique output.
    match work {
        Work::Fixed(_) => est.clamp(16384, 2_000_000) as usize,
        Work::Auto { .. } => 16384,
    }
}

pub fn run_serial<'a>(
    work: Work<'a>,
    plugins: &[Plugin],
    ctx: &Ctx,
    opts: &GenOpts,
    est: u64,
) -> Result<Vec<u8>, String> {
    let sink = output::FinalSink::with_capacity(output_hint(work, est));
    let mut em = Emitter::new(opts, sink);
    let mut sc = Scratch::new();
    match work {
        Work::Fixed(people) => engine::gen_people(people, plugins, ctx, opts, &mut em, &mut sc)?,
        Work::Auto { head, names } => {
            engine::gen_auto(head, names, plugins, ctx, opts, &mut em, &mut sc)?
        }
    }
    Ok(em.into_sink().finish())
}

pub fn run_parallel<'a>(
    work: Work<'a>,
    plugins: &[Plugin],
    ctx: &Ctx,
    opts: &GenOpts,
    threads: usize,
    est: u64,
) -> Result<Vec<u8>, String> {
    match work {
        Work::Fixed(people) => {
            let chunk_size = 512;
            let total_chunks = people.len().div_ceil(chunk_size);
            if total_chunks <= 1 || threads <= 1 {
                return run_serial(work, plugins, ctx, opts, est);
            }
            let workers = threads.min(total_chunks);
            let (tx, rx) =
                std::sync::mpsc::sync_channel::<Result<output::Batch, String>>(workers * 2);
            let cursor = AtomicUsize::new(0);
            let mut final_sink = output::FinalSink::with_capacity(output_hint(work, est));
            let mut merge_err: Option<String> = None;

            std::thread::scope(|s| {
                for _ in 0..workers {
                    let tx = tx.clone();
                    let cursor = &cursor;
                    s.spawn(move || {
                        let mut sc = Scratch::new();
                        loop {
                            let chunk_idx = cursor.fetch_add(1, Ordering::Relaxed);
                            if chunk_idx >= total_chunks {
                                break;
                            }
                            let start = chunk_idx * chunk_size;
                            let end = (start + chunk_size).min(people.len());
                            let batch =
                                output::Batch::new(chunk_idx, (end - start) * plugins.len());
                            let mut em = Emitter::new(opts, batch);
                            if let Err(e) = engine::gen_people(
                                &people[start..end],
                                plugins,
                                ctx,
                                opts,
                                &mut em,
                                &mut sc,
                            ) {
                                let _ = tx.send(Err(e));
                                break;
                            }
                            if tx.send(Ok(em.into_sink())).is_err() {
                                break;
                            }
                        }
                    });
                }
                drop(tx);

                merge_err = merge_batches(rx, total_chunks, &mut final_sink).err();
            });

            if let Some(e) = merge_err {
                return Err(e);
            }
            Ok(final_sink.finish())
        }
        Work::Auto { head, names } => {
            if names.family.is_empty() {
                return run_serial(work, plugins, ctx, opts, est);
            }
            // Aim for roughly 512 people per batch, including large given-name
            // dictionaries, instead of reserving a large product per worker.
            let chunk_size = (512 / names.given.len().max(1)).max(1);
            let total_chunks = names.family.len().div_ceil(chunk_size);
            if total_chunks <= 1 || threads <= 1 {
                return run_serial(work, plugins, ctx, opts, est);
            }
            let workers = threads.min(total_chunks);
            let (tx, rx) =
                std::sync::mpsc::sync_channel::<Result<output::Batch, String>>(workers * 2);
            let cursor = AtomicUsize::new(0);
            let mut final_sink = output::FinalSink::with_capacity(output_hint(work, est));
            let mut merge_err: Option<String> = None;

            std::thread::scope(|s| {
                for _ in 0..workers {
                    let tx = tx.clone();
                    let cursor = &cursor;
                    s.spawn(move || {
                        let mut sc = Scratch::new();
                        loop {
                            let chunk_idx = cursor.fetch_add(1, Ordering::Relaxed);
                            if chunk_idx >= total_chunks {
                                break;
                            }
                            let start = chunk_idx * chunk_size;
                            let end = (start + chunk_size).min(names.family.len());
                            let batch = output::Batch::new(
                                chunk_idx,
                                (end - start) * names.given.len() * plugins.len(),
                            );
                            let mut em = Emitter::new(opts, batch);
                            let head_part: &[Person<'a>] = if chunk_idx == 0 { head } else { &[] };
                            let part = AutoNames {
                                given: names.given,
                                family: &names.family[start..end],
                            };
                            if let Err(e) = engine::gen_auto(
                                head_part, part, plugins, ctx, opts, &mut em, &mut sc,
                            ) {
                                let _ = tx.send(Err(e));
                                break;
                            }
                            if tx.send(Ok(em.into_sink())).is_err() {
                                break;
                            }
                        }
                    });
                }
                drop(tx);

                merge_err = merge_batches(rx, total_chunks, &mut final_sink).err();
            });

            if let Some(e) = merge_err {
                return Err(e);
            }
            Ok(final_sink.finish())
        }
    }
}

// Owning the receiver disconnects blocked producers on every return path,
// before the enclosing thread scope joins them.
fn merge_batches(
    rx: std::sync::mpsc::Receiver<Result<output::Batch, String>>,
    total_chunks: usize,
    sink: &mut output::FinalSink,
) -> Result<(), String> {
    let mut pending = std::collections::HashMap::new();
    let mut next_chunk = 0;
    while next_chunk < total_chunks {
        if let Some(batch) = pending.remove(&next_chunk) {
            output::merge_batch(&batch, sink);
            next_chunk += 1;
            continue;
        }
        match rx.recv() {
            Ok(batch) => {
                let batch = batch?;
                if batch.chunk_idx == next_chunk {
                    output::merge_batch(&batch, sink);
                    next_chunk += 1;
                } else {
                    pending.insert(batch.chunk_idx, batch);
                }
            }
            Err(_) => break,
        }
    }
    Ok(())
}
