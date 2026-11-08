---
title: "Values and operators"
aliases: ["/docs/language-design/"]
weight: 3
---

UTF-8 source supports LF/CRLF. ASCII identifiers, # comments, insignificant indentation and mandatory end delimiters. Newlines separate statements; parentheses/arrays allow continuation, as does a trailing ->. No semicolons, braces or trailing commas. Strings accept escapes \n, \r, \t, \\ and \"; raw newlines and unknown escapes fail. Numbers use decimal digits with an optional fractional part; unary minus supplies negatives.

Number is finite IEEE 754 binary64 (53-bit integer precision). String, Bool, Unit and homogeneous arrays have inferred static types. Assignment declares or updates in the current scope and preserves type. Arithmetic and ordering require Number; equality compares same-type scalars. print accepts only Number, String or Bool.

Precedence high to low: call/index/member; ^ (right associative); unary -/not; */%; +/-; comparisons (non-associative); and; or; ->. Other binary tiers associate left. and/or short circuit. Division/remainder by zero and nonfinite results fail.
