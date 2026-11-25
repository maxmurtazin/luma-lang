# CAD-1 · рецепт и команда в Core ProjectStore

Локальный профиль принимает рецепт в существующий пустой Part, затем
пересчитывает его при изменении Length-параметров командой. Пример из
приложения B.1 создаёт пластину 100×60×8 мм с отверстиями Ø6 мм в X=±40 мм.
Команда B.2 задаёт override 120 мм и перемещает отверстия в X=±50 мм.
Feature ID и именованный выход сохраняются. История команды и точный принятый
пакет рецепта лежат в разных полях чекпойнта Core.
[Рецепт](../../examples/cad/plate-geometry.json) · [Команда](../../examples/cad/plate-width-command.json)

Адаптер устанавливается в отдельную чистую рабочую копию закреплённого
Living IR Core. Он использует SQLite CAS существующего `ProjectStore`,
авторитетные ревизии и штатный нативный вычислитель. Подготовка проверяет
Canonical HIR и строит нативное тело. При применении Core сверяет
аутентифицированный токен, сборку провайдера, проект, срок и ревизию. Снапшот,
метаданные рецепта, исходники, квитанция и ссылка сцены сохраняются вместе.
Повтор с тем же ID и токеном возвращает исходную квитанцию, включая повтор
после перезапуска или при параллельной попытке. Для другого токена с тем же
ID возвращается `RequestIdReuse`.

Доступ к адаптеру имеет локальный владелец с ключом проекта. Профиль
поддерживает один управляемый рецепт на Part и один именованный выход
`solid`. Команды работают с допущенным Part, видят свои предыдущие правки
и один раз запускают принятый рецепт с итоговыми overrides. Удаление
принятых features и смена публичного выхода требуют согласования ссылок.
Обычные команды редактирования Core для управляемого Part блокируются.
View V2 получает сцену из принятой квитанции, включая историческую ревизию.

## Воспроизведение

Для Linux использованы Rust 1.100.0, Core на SHA
`e8d5a465dbc943b746ce9f3607fe526f1b3a93df` и View на SHA
`a94fe6cca6f1d2982bb85e4cd4170422ba77f7b4`. Для T1 сначала соберите
web assets в отдельном checkout View по инструкции V2.
[Ревизии](../../luma-cad.lock.json) ·
[Сборка View V2](https://github.com/maxmurtazin/living-ir-view/blob/a94fe6cca6f1d2982bb85e4cd4170422ba77f7b4/docs/core-v2.md)

```sh
python3 -B tools/cad/prepare-core-project.py "$CORE" --view "$VIEW"
cargo test --manifest-path "$CORE/Cargo.toml" -p living-cad-viewer \
  --features luma-cad,living-view-v2 luma_cad1_recipe --lib
cargo build --manifest-path "$CORE/Cargo.toml" -p living-cad-viewer \
  --features luma-cad,living-view-v2,checkpoint-fault-injection --bin luma-cad-project
python3 -B tools/cad/test-core-project.py "$CORE/target/debug/luma-cad-project" \
  --scene /tmp/luma-accepted-scene.json
WEBGPU_SOFTWARE=1 node tools/cad/test-view-scene.mjs "$VIEW" /tmp/luma-accepted-scene.json
```

Для T0 исключите `--view "$VIEW"`, `living-view-v2` и вывод сцены. Тест
создаёт временный проект, генерирует ключ и завершает дочерний процесс в
контрольных точках SQLite. Feature `checkpoint-fault-injection` используется
для проверки сбоев. Для обычной локальной сборки достаточно `luma-cad` и
`living-view-v2`, если подготовлен View.

## Локальный CLI

Владелец задаёт `LUMA_CAD_PROJECT_KEY` как 64 hex-символа. Команды получают
путь к базе и UUID проекта первыми двумя аргументами. `preview` создаёт
новый файл токена. `apply` принимает токен и идентификатор операции из
32 lowercase hex-символов.

| Команда после `DB PROJECT` | Результат |
| --- | --- |
| `init PART OPERATION` | Создаёт локальную SQLite-базу с пустым Part |
| `preview recipe PART REVISION BUNDLE TOKEN` | Проверяет рецепт, сохраняет токен |
| `preview command PART REVISION BUNDLE TOKEN` | Выполняет команду в overlay, проверяет пересчёт принятого рецепта |
| `apply TOKEN OPERATION` | Применяет токен с CAS и возвращает квитанцию |
| `receipt PART OPERATION` | Находит квитанцию после сбоя или потери ответа |
| `scene PART OPERATION SCRATCH` | Возвращает пакет View V2 из принятой ревизии |

## Объём квалификации

Профиль закрывает контрольную цепочку recipe → SQLite commit → command
override → SQLite commit → View V2. Локально проверены обе нативные формы,
исторический pin, отмена, устаревшая база, подмена токена, конкурирующий
повтор, сбой до и после COMMIT и восстановление после потери ответа.
На T1 Chromium 159.0.8154.6 со SwiftShader отрисовал пакет принятой
ревизии 3. CI запускает нативный и аварийный сценарии T0.
[Интеграционный тест](../../integrations/living-ir-core/project.rs) · [Проверка CLI и SQLite](../../tools/cad/test-core-project.py)

Для полного выхода CAD-1 остаются, в частности, программные задания и
durable job outbox (AT-039–AT-041, AT-082, AT-086), межрецептурные зависимости
и атомарный пересчёт closure (AT-090), управляемое удаление и detach
(AT-019–AT-021), а также единая приёмка через UI, CLI и MCP и полный
профильный benchmark (AT-076, AT-079, AT-081). Остальные непроверенные
сценарии §32 сохраняют статус `not_run`.
