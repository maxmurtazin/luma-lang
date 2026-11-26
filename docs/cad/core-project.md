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
поддерживает один рецепт на Part и один геометрический выход `solid`.
Открытый Length-параметр другого рецепта можно связать с Length-параметром
Part. Пропущенная `revision` задаёт `follow_current`, число из
`recipe-info` закрепляет `exact_pinned`. Командный overlay читает
связанный параметр и отклоняет его прямую запись. При пересчёте потребитель
`follow_current` получает кандидат производителя. Исторический pin
сохраняет значение принятой ревизии. Все изменённые Part, постановки
заданий и квитанция принимаются одним SQLite CAS. Ошибка любого потребителя
отклоняет кандидат целиком.

Ссылки сборки защищают прежний выход и удаляемые features. Удалённые ключи
остаются в реестре. Полный detach сохраняет нативные определения, ID и
исходник, освобождает Part для штатного редактирования и перечисляет
освобождённые сущности и связи. Рецепты с `follow_current` должны
освободить связь перед detach производителя. View V2 читает точную
принятую ревизию по квитанции.

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
| `create-part PART REVISION OPERATION` | Добавляет пустой Part в существующую базу |
| `preview recipe PART REVISION BUNDLE TOKEN [JOB_KEY ...]` | Проверяет рецепт, сохраняет токен |
| `preview command PART REVISION BUNDLE TOKEN [JOB_KEY ...]` | Выполняет команду в overlay, проверяет пересчёт принятого рецепта |
| `inspect` / `recipe-info PART` | Показывает ревизии проекта и принятого рецепта |
| `bind PART REVISION BINDINGS TOKEN [JOB_KEY ...]` | Готовит замену типизированных связей |
| `detach PART REVISION TOKEN` | Готовит полный detach и возвращает отчёт |
| `apply TOKEN OPERATION` | Применяет токен с CAS и возвращает квитанцию |
| `receipt PART OPERATION` | Находит квитанцию после сбоя или потери ответа |
| `job PART KEY` / `work` | Читает статус по ключу или публикует одно принятое задание |
| `job-cancel JOB` / `job-retry JOB ATTEMPT` | Отменяет задание или допускает новую попытку |
| `job-definition JOB` | Возвращает опубликованное каноническое определение и его digest |
| `scene PART OPERATION SCRATCH` | Возвращает пакет View V2 из принятой ревизии |

Пример файла для `bind`. `producer` содержит UUID Part, которому
принадлежит публикуемый параметр.

```json
[
  {"target": "width", "producer": "00000000-0000-4000-8000-000000000002", "output": "width"}
]
```

Нативный профиль заданий публикует неизменяемое каноническое определение
Part. Программа ставит его в очередь через
`jobs.submit("definition", jobs.task("part_definition"))`. Допущенный
`JOB_KEY` открывает `jobs.job(key)` для `jobs.status` и `jobs.result`
по захваченной квитанции. `apply` и `receipt` возвращают соответствия
`jobs` для основного Part и `dependent_jobs` по UUID остальных Part.
[Очередь заданий](job-queue.md)

## VS Code и MCP

Команда **Luma: Preview CAD Bundle** использует локальный
`luma-cad-project`, настроенный для пользователя через
`luma.cadProjectExecutable`. VS Code показывает кандидат и запрашивает
подтверждение перед применением. CLI и расширение получают ключ проекта
из одной переменной среды. [Настройка расширения](../../editors/vscode/README.md)

Локальный MCP работает через stdio с тем же CLI и фиксированным проектом.
`prepare_recipe` и `prepare_commands` возвращают временный идентификатор
подготовки. `commit` принимает идентификатор кандидата и операции.
После перезапуска `receipt` восстанавливает результат по Part и операции.

```sh
python3 -B tools/cad/project-mcp.py /path/to/luma-cad-project /path/to/project.db PROJECT_UUID
```

## Квалификация

Ранее локально прошла цепочка рецепт → команда → View V2 для одной
пластины. Chromium 159.0.8154.6 со SwiftShader отрисовал принятую ревизию 3.
Эта проверка относится к прежнему профилю. В новом профиле тест Luma
проверяет подготовку зависимых рецептов, ограничение удаления и detach.
Независимый SQLite-тест проверяет откат модели, схемы очереди и задания.
Тесты MCP и VS Code проверяют транспорт на заглушках. В VS Code отдельно
проверены подтверждение применения и восстановление квитанции при потере
ответа. [Тесты Luma](../../tests/cad_reconciliation.rs) ·
[Тест MCP](../../tools/cad/test-project-mcp.py)

Для сквозного прогона сценарий CLI покрывает rollback модели и заданий при
сбое до COMMIT, восстановление соответствий после COMMIT, `follow_current`,
`exact_pinned`, отказ при выходе потребителя за границы и detach.
Квалификация обновлённого адаптера на приватном Core, единый сценарий через
VS Code, CLI и MCP и лимиты полного профиля ещё имеют статус `not_run`.
[Сценарий CLI](../../tools/cad/test-core-project.py) ·
[Ручной CI](../../.github/workflows/cad-core.yml)

Linux-бенчмарк T0 записывает p50, p95, пиковый RSS дочерних процессов и
идентификаторы сборки и lock. Замеры обновлённого нативного профиля и
отдельная квалификация рендеринга T1 ожидают прогона.

```sh
python3 -B tools/cad/benchmark-project.py /path/to/luma-cad-project --build BUILD_ID
```
