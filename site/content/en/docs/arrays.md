---
title: "Arrays"
aliases: ["/docs/arrays/"]
weight: 5
---

Arrays are homogeneous and zero indexed, including nested arrays. Copies and arguments are recursively by value. Indexed writes require a local binding. Indices must be nonnegative integral Numbers within bounds; negative, fractional and out-of-range indices fail at runtime. Empty arrays require enough type context. There is no append, length, slicing or general collection API.

~~~text
a = [[1, 2]]
b = a
b[0][1] = 9
print(a[0][1])
print(b[0][1])
~~~
