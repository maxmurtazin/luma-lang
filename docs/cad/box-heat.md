# CAD-4 · Стационарная теплопроводность Box

Профиль `cad4-box-heat-v1` использует прямоугольный Box из принятой
ревизии Core Part. Поддерживаются перенос и оси, параллельные мировым.
Сфера, повёрнутый Box и другие формы отклоняются при проверке
источника. Отчёт сохраняет размеры и мировое начало сетки в метрах.

Исследование задаёт число ячеек, идентификатор материала, его
поколение, теплопроводность в Вт/(м·К) и две температуры в К на
гранях X. Четыре остальные грани изолированы. Внутреннее
тепловыделение равно нулю. Значение теплопроводности и поколение
задаёт вызывающая сторона, а очередь фиксирует их вместе с
постановкой. Лимит — 32 ячейки на ось и 4096 ячеек всего.

`PROJECT_CLI` собирается по [инструкции проектного адаптера](voxel-project.md).
`SIM` указывает на checkout Living Simulation из [CAD lock](../../luma-cad.lock.json).

```sh
cargo build --locked --manifest-path "$SIM/Cargo.toml" -p simulation-cli
LSIM="$SIM/target/debug/lsim"
"$PROJECT_CLI" "$DB" "$PROJECT" heat-source "$PART"
"$PROJECT_CLI" "$DB" "$PROJECT" heat-submit "$LSIM" "$REQUEST" \
  "$PART" "$PART_REV" "$DEFINITION_HASH" examples/cad/box-heat-study.json
"$PROJECT_CLI" "$DB" "$PROJECT" heat-work "$LSIM"
"$PROJECT_CLI" "$DB" "$PROJECT" heat-export "$JOB" box.lsim > box-heat-report.json
"$PROJECT_CLI" "$DB" "$PROJECT" heat-cell "$LSIM" "$JOB" 0 0 0
"$PROJECT_CLI" "$DB" "$PROJECT" heat-boundary "$LSIM" "$JOB" min_x
```

`REQUEST` — UUID команды, `JOB` — номер из квитанции. Для проекта
используется `LUMA_CAD_PROJECT_KEY`. `heat-receipt` читает квитанцию,
`heat-status` показывает состояние. После сбоя отмените активную
попытку через `heat-cancel JOB` и запустите `heat-retry JOB ATTEMPT`.
Экспорт защищён от перезаписи. Отчёт сохраняет постановку, материал,
хеш решателя, SHA‑256 архива, невязку и баланс. `heat-cell` открывает
исторический архив с тем же решателем.

Для Box 40 × 30 × 20 мм с температурами 300 и 340 К контроль даёт
поток ±1,2 Вт и 302,5 К в первой ячейке. Удвоение проводимости во
втором поколении материала даёт ±2,4 Вт. Решатель сохраняет
квалификацию `Experimental`. Профиль покрывает нативный CLI
проекта. Отдельные профили работают на том же Box. [Диффузия и постоянный ток](box-transport.md)

```sh
LUMA_SIM_CLI="$LSIM" LUMA_VOXEL_PROVIDER="$CLI" LUMA_VOXEL_CLI="$VOXEL_CLI" \
  cargo test --locked --manifest-path "$BUILD/project/Cargo.toml" \
  --features checkpoint-fault-injection,cad4-thermal-tests --test project
```

### Команды Luma

`heat-submit-luma` принимает путь к пакету команды после файла исследования.
`sim.pinned_study()` возвращает исследование, переданное проектному CLI.
`sim.solve` объявляет одну задачу, а после успешной проверки CLI ставит её
в очередь. Постановка и дайджест команды сохраняются вместе.

`heat-cell-luma` и `heat-boundary-luma` принимают пакет команды
после индекса ячейки или имени грани. `sim.query` читает закреплённый
результат с величиной и селектором из вызова CLI. Команды работают в
режиме `command`. Запрос передачи результата в Living View возвращает
`UnsupportedCapability` через `heat-view JOB`.

[Пакет для sim.solve](../../examples/cad/box-solve-command.json) ·
[Пакет для sim.query](../../examples/cad/box-query-command.json)

[Манифест профиля CAD-4](../../conformance/cad/cad4-box-heat-v1.json)
