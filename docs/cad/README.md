# CAD-0 · контракты и совместимость

[Спецификация LCAD-SPEC-001 v1.0](../../spec/cad/Luma-CAD-Development-Specification-v1.0-RU.md) ·
[Возможности (RU)](capabilities-ru.md) · [Capabilities (EN)](capabilities-en.md) ·
[Lock](../../luma-cad.lock.json) · [CAD-1 · host API](host-api.md)

- [ADR 001 · Компоненты и authority](../../spec/cad/adr-001-authority.md)
- [ADR 002 · Каноническая граница Sketch/Profile](../../spec/cad/adr-002-sketch-profile.md)
- [ADR 003 · Native type schemes и host registry](../../spec/cad/adr-003-native-types.md)

CAD-0 закрепляет исходники и схемы DTO. Выбранный provider-комплект — Core и
Living IR View с адаптером V2. В нём подтверждается просмотр mesh-сцен Core.
Для Luma 0.1.3 доступны локальный маршрут CAD-1 и квалифицированный
профиль CAD-2. [Реестр](../../conformance/cad/registry-v1.json) указывает
статусы операций и гранулярность проверки. Структурный экспорт S1
поддерживает чтение. S2 validated edits остаётся отдельным draft.

## Закреплённый комплект

| Провайдер | Граница и точки входа |
| --- | --- |
| Core | `FeatureSpec::plate`, `FeatureSpec::hole`, `prepare_rebuild`, `prepare_transaction`, `DomainRegistry.schemas`, `living_sketch::build_profiles`. SQLite multi-part хранит исходные определения, ревизии и pins. Маршрутизация из Luma требует нового адаптера. |
| View | Патч V2 и `integrations/core/render.rs` с projection/picking по source pins, WebAssembly `create_mesh_viewport`, `scripts/check-v2.mjs`. V2 выбирает mesh по текущему принятому снимку. |
| 2D | `lir2d-runtime` выполняет автономные команды и встроенные команды с регистрацией и Core CAS. |
| B-rep | `brep-core` и `integrations/living-ir-preview` предоставляют normal extrusion и пилот публикации. M7 planar Boolean имеет отдельные ограничения. |
| Voxel | `schemas/capabilities-v1.json` и `FieldEvaluator` задают ограничения sampling. View V2 использует mesh; volume renderer отсутствует. |
| Simulation | Интеграционный Cargo manifest закрепляет Core `dce28055d404c7fad21749636554acf8b7a0d0b2`. Для выбранного Core `e8d5a46` совместимость этого адаптера не подтверждена. |
| Elements | `docs/ADAPTER-CONTRACT.md` и `src/capabilities.rs` описывают автономные операции. Embedded запись требует registered state. |
| Studio | `S5-RASTER-EXPORT.md` и `S6-LIVING-IR-PREVIEW.md` описывают Cycles CPU, standalone presentation и закреплённые источники. |

Lock связывает точные ревизии с SHA-256 файлов зависимостей и адаптера. Build
fingerprint вычисляется через LCAN/1 в доменах `luma.cad.build.provider`
и `luma.cad.build.adapter`. Вход исключает собственное поле `build_fingerprint`.
Fingerprint адаптера включает fingerprints выбранных провайдеров. Это идентичность входов сборки. Метрики и версия среды записаны отдельно.
Квалификация Luma и наблюдаемые результаты upstream разделены в lock и реестре.

## DTO и проверка

[Wire v1](../../schema/cad/wire-v1.json) задаёт envelopes, typed results и ссылки.
[Core state v1](../../schema/cad/core-state-v1.json) задаёт предлагаемую форму
зарегистрированного агрегата. [Sketch/Profile v1](../../schema/cad/sketch-profile-v1.json)
задаёт общую границу 2D и B-rep. Проверки доступа, scope, топологии, размеров и
истории выполняются во время admission и prepare на последующих этапах.
В [LCAN vectors](../../conformance/cad/lcan-vectors-v1.json) теги `$fixture_*`
обозначают типизированные правила fixture и не входят в результат канонизации.

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install jsonschema==4.23.0
python -B tools/cad/validate.py
python -B tools/cad/validate.py --write-docs
python -B tools/cad/validate.py --provider core=/path/to/core --provider view=/path/to/view
```

[Матрица, команды воспроизведения и результаты](evidence.md)

## CAD-2

[Манифест выпуска профиля](../../conformance/cad/cad2-profile-v1.json) ·
[Публикация и измерение нормальной B-rep экструзии](brep-project.md) ·
[Самостоятельный Living IR 2D](standalone-2d.md) · [Встроенный 2D](embedded-2d.md)
