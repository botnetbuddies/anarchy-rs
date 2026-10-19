# Compatibility shim for running the Ruby oracle on Ruby >= 3.2.
#
# The original `Name#format` relies on `Object#=~` returning nil for
# non-String receivers (e.g. an Array produced by an earlier expansion branch
# in the same block iteration). Ruby removed `Object#=~` in 3.2, so on modern
# Rubies any substitution other than `%D` raises NoMethodError.
#
# Defining it back reproduces the semantics the tool was written against
# (Ruby <= 3.1) and is used only as the parity oracle for substitution cases.
# See PERFORMANCE.md ("Oracle & Ruby version") for the full rationale.
class Object
  def =~(other)
    nil
  end
end
