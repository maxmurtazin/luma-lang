---
title: Справочник синтаксиса
weight: 14
---

Текущая ограниченная грамматика. Переводы строк разделяют инструкции, end завершает блоки. Компактная печать A4 остаётся целью; физическая разбивка на страницы не проверена.

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
