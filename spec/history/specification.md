---
title: Luma 0.1 — Draft 001
weight: 12
---

**Draft 001 · November 2026 · Syntax implemented in 0.1B; bounded static semantics implemented in 0.1C.** See [implementation boundaries](../semantics/) for adopted semantic proposals and conservative exclusions. Runtime contracts below remain proposals. No interpreter conformance has been executed.

## Authority and status

This formalizes the conversation's “Luma 0.1 — Language Specification”, Draft 001. Agreed rules below preserve that design. Rules explicitly called **0.1A bounded proposal** close a small parsing or checking gap for fixtures; their lexical and grammatical subset is adopted in 0.1B; semantic proposals remain reviewable. See [frontend](../frontend/). Deferred features have no implied behavior. The compact syntax reference has a one-A4-page goal; this full specification is intentionally separate.

## Lexical rules — 0.1A bounded proposal

UTF-8 source; LF and CRLF are equivalent. ASCII identifiers use [A-Za-z_][A-Za-z0-9_]*; keywords cannot be identifiers. Keywords: fn return end match object use pub true false and or not. # starts a comment outside a string. Strings use double quotes and escapes \n, \r, \t, \\, and \"; unknown escapes and raw newlines are lexical errors. Numbers are decimal digits with an optional fractional part (digits on both sides of the dot). Negative values use unary minus. Braces and semicolons are rejected. Indentation is insignificant; end is mandatory.

A newline separates statements and match arms. Inside parentheses or square brackets it is whitespace. A line ending with -> continues the pipeline onto the next line. Other implicit continuation is excluded. Blank/comment lines are ignored. EOF may terminate the last statement. Headers require a newline before their body.

## Grammar — 0.1A bounded proposal

EBNF: NL means one or more significant newlines; lists may be empty unless marked +. Whitespace between tokens is ignored. Productions below omit blank lines.

~~~text
program     = { top NL } ; # EOF also terminates final top form
top         = "use" name | ["pub"] definition | statement ;
definition  = function | object | name "=" expression ;
function    = "fn" name "(" [names] ")" NL { statement NL } "end" ;
object      = "object" name NL { (function | name "=" expression) NL } "end" ;
statement   = target "=" expression | "return" [expression] | expression ;
target      = name { "[" expression "]" } ;
names       = name { "," name } ;
expression  = pipeline ;
pipeline    = logical_or { "->" stage } ;
stage       = qualified [ "(" [arguments] ")" ] ;
qualified   = name { "." name } ;
arguments   = expression { "," expression } ;
logical_or  = logical_and { "or" logical_and } ;
logical_and = comparison { "and" comparison } ;
comparison  = additive [ ("==" | "!=" | "<" | ">" | "<=" | ">=") additive ] ;
additive    = product { ("+" | "-") product } ;
product     = unary { ("*" | "/" | "%") unary } ;
unary       = ("-" | "not") unary | power ;
power       = postfix [ "^" unary ] ;
primary     = number | string | "true" | "false" | qualified
            | "(" expression ")" | "[" [arguments] "]" | match ;
postfix     = primary { "(" [arguments] ")" | "[" expression "]" } ;
match       = "match" expression NL arm { NL arm } NL "end" ;
arm         = pattern ":" expression ;
pattern     = literal | ("==" | "!=" | "<" | ">" | "<=" | ">=") literal | "*" ;
~~~

Operator tiers, highest first: postfix calls/index/member access; ^ (right associative); unary - and not; * / %; + -; comparisons == != < > <= >= (non-associative); and; or; -> (left associative). Other binary tiers associate left. Power binds tighter than unary minus; its right operand may have unary minus. literal in patterns is Number (optionally negative), String, or Bool; ordered patterns require Number. Match-arm ':' is separate from the pipeline arrow. Assignment is a statement, never an expression. Nested match expressions consume their own end. No trailing commas, nested declarations in functions, or arbitrary member assignment in this bounded grammar.

## Values, inference, calls and scope

Agreed: Number, String, Bool, Unit, homogeneous Array&lt;T&gt;, callable signatures and named structural object scopes. First assignment declares in the current lexical scope; subsequent assignments there preserve type. Function assignment never implicitly writes an outer binding. Parameters pass by value, including arrays (copy-on-write is permitted internally). Unknown names and duplicate function names are static errors. No overloads, classes, let/var, async/await, for/while, or braces.

Functions have one monomorphic signature inferred from bodies and reachable usage across the checked import graph. Solve constraints before execution, including recursive calls; unresolved or conflicting types are static errors. There are no implicit conversions between Number, String and Bool. Named functions may be passed as callbacks. Anonymous functions and generic user functions are deferred. Library signature metavariables describe per-call library contracts, not user polymorphism.

**0.1A bounded proposal:** parameters are locals; reading a local before its first assignment fails even if an outer name exists. A name assigned anywhere in a function is local throughout it. Functions may read lexical outer bindings. Names share a namespace within a scope; duplicate parameters/declarations fail. Top-level function names are available for forward calls, but reading values before initialization fails statically. Match arms are expressions and introduce no bindings. Reaching function end yields Unit; bare return yields Unit; all possible return paths must unify. Return outside a function fails. Call arity is exact; evaluate callee, then arguments left to right, each once. and/or short-circuit and require Bool. Assignment evaluates its right side before storing; indexed assignment evaluates target indexes left to right before its right side.

## Numbers and errors

Agreed: Number is one visible type; signed 64-bit integers and binary64 may represent it internally. Integer overflow is an error; integer / produces a floating result. Division by zero and invalid indexes are runtime errors. Diagnostics retain file and source span whenever available. There is no try/catch syntax. Static failure prevents execution of the whole program.

**0.1A bounded proposal:** + - * on two integer representations use checked integer operations; / converts operands to binary64. Fixtures use exactly representable small values only. Arithmetic requires Number; string concatenation is deferred, so examples use print with a String directly. Comparisons of equality require identical types; ordered comparison requires Number. Indexes must be integral, nonnegative Number values. Failures propagate to the caller and terminate a run; partial successful results are not returned.

Mixed-representation rounding, % on floats/negative values, ^ overflow/domain behavior, nonfinite values and equality for compound values remain unresolved; no fixture claims those results. They block full numeric-runtime acceptance, not the bounded syntax exercise.

## Pipeline and match

Agreed: x -> f(a) means f(x, a); x -> f means f(x). Stages execute left to right, once. The target must be callable; -> never assigns. result = 10 -> square -> print stores Unit. Match evaluates its subject once, tests branches in source order, and evaluates only the first selected arm. Literal patterns mean equality; comparator patterns compare the subject to the literal. Branch result types must unify.

**0.1A bounded proposal:** every match requires exactly one final * arm, even for Bool; no exhaustiveness theorem beyond that fallback is needed. Earlier arms after a wildcard and mismatched pattern types fail statically. No guards, binding or destructuring patterns.

## Arrays, repetition and library contracts

Agreed: arrays have dynamic length and homogeneous elements; zero-based indexes; out-of-range access fails at runtime. Empty literals need sufficient context to infer an element type. Parameter updates do not change caller arrays. Local indexed mutation is allowed. Slicing and resizing syntax are deferred.

Bounded library contracts (proposed APIs, unimplemented): map(Array&lt;T&gt;, fn(T):U):Array&lt;U&gt;, parallel has the same shape plus the effect restriction below; sum(Array&lt;Number&gt;):Number; print(String or Number or Bool):Unit is an effectful library family, not user overload syntax. repeat(T, fn(T):T, fn(T):Bool):T checks the predicate before each step and returns the current value when true. map is eager, sequential and input ordered. repeat is sequential and may not terminate. filter, reduce, find, file APIs and conversions are planned but their exact contracts remain open.

## Objects and modules

Agreed: object Building ... end defines a named structural scope of bindings and functions. Building.volume(120) is an ordinary qualified function call with no implicit self. An object function may read sibling height lexically. Objects have no constructors, instances, inheritance or dispatch. Object identity, first-class object references and cloning are outside 0.1.

One .luma file is one module; use math binds the module namespace and accesses math.name. pub exports a definition; unmarked definitions are private. Imports initialize once; cycles fail statically.

**0.1A bounded proposal:** use statements precede other top-level forms. Only sibling files addressed by a single identifier are in the fixture resolver boundary; no aliases, directory/package paths or implicit standard-module lookup. Import dependencies initialize before importers; independent imports follow source order. Top-level statements execute in source order. pub can mark a function, binding or object. Exported objects expose their declared members; private modules remain inaccessible through them. Imported and object members are read-only from outside their declaring scope; assignment to qualified names fails. Missing imports, cycles, private access and ambiguous names fail before execution. Inference spans all reachable files.

## Explicit synchronous parallel effects

Agreed: parallel maps a callback over an array, returns only after completion or failure, and preserves input order. Worker count, scheduling and speedup are unspecified. Workers cannot perform console/file I/O or modify captured external state. Conservative static effect analysis follows transitive calls.

**0.1A bounded proposal:** callbacks may mutate their own locals and by-value parameters. Reads of external bindings are permitted only when analysis proves they cannot change during the operation. Unknown callback effects, I/O through helpers, or external writes cause static rejection. A function-local assignment with an outer namesake remains local. No user thread API exists. Nested parallel execution strategy, cancellation timing, selection among simultaneous worker errors and resource budgets remain open; do not assert a particular failing worker or timing. No partial result escapes on failure.

## Implementation boundary and acceptance

Planned Rust pipeline: lexer → parser → AST → module/name resolution → type/effect analysis → interpreter → runtime/library. Source locations survive all stages. Planned CLI: luma check, luma run, luma version. VS Code structural, dependency and control-flow graphs derive from analyzed structure; no flow/goto syntax is added.

0.1A is **bounded GO for lexer/parser implementation review**, subject to approval of the labeled bounded proposals. Full runtime acceptance remains pending numeric and library decisions and an interpreter. Native compilation, WebAssembly, object identity, anonymous functions, generic user functions, structured recovery and Luma Mark mapping remain deferred. Memory strategy, recursion/resource limits and Unicode text indexing remain implementation/design questions. See the conformance page for executable-concept cases and actual validation evidence.



0.1D2 / distribution 0.1.1: `map`, `filter`, `reduce`, `sum`, `repeat`, and CPU-threaded `parallel` now execute. The current contract and bounds are in [standard-library](../standard-library/) (Hugo docs: `site/content/docs/standard-library.md`); this supersedes earlier deferred-runtime statements. A4 pagination remains unverified.
