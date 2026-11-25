# CAD-1 · план пластины и отверстий

Профиль `recipe-geometry-v1` поддерживает `cad.plate`, `cad.through_hole`,
`cad.output`, `view.from_body` и `view.show`. Пластина центрирована
по X, Y и Z. Центр отверстия задан в XY, вход находится на верхней
грани. Ось сквозного отверстия направлена вдоль −Z. Для размеров и
координат принимаются Quantity, ParamRef и ParamExpr размерности Length.

```sh
cargo run --locked -- cad check examples/cad/plate-geometry.json --mode recipe
cargo run --locked -- cad plan examples/cad/plate-geometry.json --mode recipe
```

`cad plan` возвращает JSON `luma.cad.geometry-plan/1` с хешем пакета,
выражениями, параметрами, порядком операций, числовыми аргументами в SI
и намерениями показа. Ссылки на тела действуют в одной подготовке. В
результате они представлены логическими ключами операций. Ключи выходов
относятся к отдельному пространству имён. Рецепт из приложения B.1
строит цепочку `blank → left_hole → right_hole` и публикует в плане
выход `solid`. [Пример пакета](../../examples/cad/plate-geometry.json)

Предварительная проверка отклоняет неположительные размеры и диаметры,
зазор менее 1 мм до края или соседнего отверстия, превышение лимита
64 отверстий на тело и повторные ключи. `luma::cad::prepare_geometry_edits`
применяет пакет overrides к копии снимка и запускает рецепт один раз
с конечными значениями. Если пересчёт дал ошибку, весь кандидат отклоняется.

## Проверка на Core

Тест использует отдельный `DefinitionStore` в памяти и ревизию Core
из lock. Пластину он переводит в `FeatureSpec::plate`, отверстия —
в `FeatureSpec::hole` с `ThroughAll` и `OppositeNormal`. Параметры
размеров передаются через Symbolic Length slots. Ссылки `base` разрешаются
в `Solid` предыдущей операции в том же снимке. Core строит нативное тело
и проверяет материал около отверстий на трёх отметках по толщине, ширину
до и после override, а также наличие материала на месте старого отверстия.
Прогон на Linux с Rust 1.100.0 прошёл для ширины 100 и 120 мм.

```sh
python3 -B tools/cad/check-core-geometry.py /path/to/clean/core-checkout
```

Интерфейс `geometry-plan/1` служит для локальной подготовки. Применение
через Core доступно в отдельном локальном профиле ProjectStore.
[Адаптер проекта и CAS](core-project.md) · [Контракт CAD-0 и lock](README.md)
