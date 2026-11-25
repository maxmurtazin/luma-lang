# ADR 001 · Размещение и полномочия записи

Решение принято для CAD-0. Версии хранения ниже зарезервированы до реализации CAD-1.

| Компонент | Путь | Граница |
| --- | --- | --- |
| `luma-host-api` | `luma-lang/crates/luma-host-api` | Descriptor, эффекты, handles и бюджеты, без CAD-зависимостей |
| `luma-cad-contract` | `luma-lang/crates/luma-cad-contract` | Wire DTO, schemas, LCAN, HostDescriptor и идентификаторы операций |
| `luma-cad-engine` | `luma-lang/crates/luma-cad-engine` | Reconciliation, planner и валидация; зависит от host API и contract |
| Core adapter | `living-ir-core/integrations/luma-cad` | Снимок, подготовка, CAS, восстановление, receipts и outbox; отдельный workspace потребителя |
| View V2 bridge | `living-ir-view/integrations/core` | Публикация производной сцены из закреплённого снимка |
| CLI/editor | `luma-lang/src` и `luma-lang/editors/vscode` | Вызов host и source mapping; legacy API и S1 schema v1 сохраняются |

Движок зависит от абстракции host. Core adapter связывает её с Core и contract.
Пакет базового интерпретатора не включает runtime providers в граф Cargo.
Владельцы standalone 2D, Elements и Studio сохраняют собственные authority.
Один commit имеет одну authority. Изменения через разные хранилища планируются как отдельные операции.

Core хранит типизированный агрегат [`recipe-instance-state/1`](../../schema/cad/core-state-v1.json)
под идентификатором `luma.cad.recipe-instance-state/1`. Точка валидации
видна в upstream `DomainRegistry.schemas`; адаптер CAD-1 регистрирует строгий
валидатор определения и включает запись только после проверки durable transaction
boundary в ProjectStore. UUID экземпляра и entity IDs сохраняются, включая tombstones.
На сохранении Core проверяет принадлежность StableKey, типы override, уникальность
живых entity IDs и согласованность output mappings, а также все CAS-guards.
Квитанция и durable job intents пишутся с определением и accepted source pin в
одной транзакции. Черновик исходника принадлежит source workspace. View владеет
сессионным отображением, а Studio — presentation state.
