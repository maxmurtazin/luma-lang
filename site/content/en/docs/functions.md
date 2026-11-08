---
title: "Functions and scope"
aliases: ["/docs/functions/"]
weight: 4
---

Named functions have exact arity, lexical captures, one monomorphic inferred signature, recursion and explicit return. A function without a value return produces Unit. Parameters and arrays pass by value. Assignment inside a function creates/updates a local binding, not an outer variable. Reads before local initialization fail; captures must be initialized when declared and called. Nested function declarations, arbitrary indirect calls and builtin function values are unsupported. Named callbacks are accepted by library contracts.

~~~text
fn square(x)
    return x * x
end
print(square(4))
~~~

**Expected output:**

~~~text
16
~~~
