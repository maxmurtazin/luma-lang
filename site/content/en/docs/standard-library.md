---
title: "Standard library"
aliases: ["/docs/standard-library/"]
weight: 9
---

All calls below execute in 0.1.3. Pipelines insert the first argument; local declarations can shadow builtin names.

| Call | Contract |
| --- | --- |
| print(value) | Print a scalar or nested arrays of scalars and newline; return Unit |
| map(array, f) | Unary callback, input order, homogeneous result |
| filter(array, predicate) | Unary Bool predicate, preserve order |
| reduce(array, initial, f) | Left fold f(accumulator, element); empty returns initial |
| sum(array) | Sequential Number addition from positive zero |
| repeat(seed, step, done) | Test done before each step; return first satisfying state |
| parallel(array, f) | Synchronous isolated CPU mapping, results in input order |

Callbacks are monomorphic; reduce may use an accumulator type different from the element. sum never reassociates arithmetic and rejects nonfinite results. Sequential callbacks may print; failure stops the operation and discards all buffered output.

repeat allows at most 10,000 transitions and a final predicate test; a true initial predicate invokes no step. The ordinary 100,000-expression budget and depth 32 may fail earlier. No convergence guarantee.


~~~text
fn square(x)
return x*x
end
fn even(x)
return x%2==0
end
fn add(a,b)
return a+b
end
fn step(x)
return x+1
end
fn done(x)
return x>=5
end
a=[1,2,3,4] -> map(square) -> filter(even)
print(a[0])
print(a[1])
print(a -> reduce(10,add))
print([1,2,3,4] -> parallel(square) -> sum)
print(0 -> repeat(step,done))
~~~

**Expected output:**

~~~text
4
16
30
30
5
~~~

Arrays use `[a, b]`; nested strings use JSON quoting and escaping. Scalar strings print without quotes. Functions, objects and Unit cannot be printed.
