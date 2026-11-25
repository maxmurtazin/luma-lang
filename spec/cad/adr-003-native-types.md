# ADR 003 · Схемы типов и единый host registry

Решение принято для CAD-0. Интеграция checker/runtime относится к CAD-1.

Реестр запечатывает namespace до разбора CAD imports. Если имя host module
совпадает с sibling `name.luma`, CAD checker выдаёт `ModuleNameCollision`.
Legacy resolver продолжает обрабатывать локальные импорты.

Мономорфный descriptor задаёт ordered parameters, result, effects, modes и
phases. Native scheme вводит связанные переменные dimensions и kinds,
ограничения равенства, результат с точной формулой размерности/вида и
явный список допустимых поднятий в `ParamExpr`. Checker создаёт свежую
подстановку при каждом прямом вызове. Runtime проверяет ту же
инстанциацию и digest descriptor set до исполнения.
Такие схемы не становятся значениями или пользовательской перегрузкой.

Дескриптор версии 1 описан в [wire schema](../../schema/cad/wire-v1.json).
Реестр содержит идентификатор и major операции. Неизвестные версии и
несогласованный fingerprint блокируют admission до мутации.
Typed ExecutionResult представляет outputs, diagnostics, source map,
plan и intents. `print` остаётся текстовым каналом.
