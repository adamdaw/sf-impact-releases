<!-- sf-impact -->
## Salesforce impact

| Head commit | Confidence | Selected tests | Untested symbols | Possible tests | Changed symbols |
| --- | --- | ---: | ---: | ---: | ---: |
| `62ceb99bbd58b8a8daeda9303c0e4fa924b91b96` | partial | 3 | 0 | 70 | 4 |

### Partial confidence (1 reasons)

Grouped by kind, input and file. Full list: impact.json artifact.

- `unmodelled_file` (changed), `nebula-logger/core/main/log-management/objects/LogEntry__c/listViews/MyGeneratedLogEntries.listView-meta.xml`: 1 reasons; first lines: unknown

### Selected tests (3)

- `LoggerHomeHeaderController_Tests` (`nebula-logger/core/tests/log-management/classes/LoggerHomeHeaderController_Tests.cls:6`): `caller`; seed `Logger.CURRENT_VERSION_NUMBER`
- `LoggerTestConfigurator` (`nebula-logger/core/tests/log-management/utilities/LoggerTestConfigurator.cls:14`): `direct`; seed `LoggerEndUser`
- `Logger_Tests` (`nebula-logger/core/tests/logger-engine/classes/Logger_Tests.cls:6`): `caller`, `convention`, `direct`; seed `Logger.CURRENT_VERSION_NUMBER`

### Untested changed symbols (0)

None.

### Changed symbols (4)

- `LogEntry__c.WasLoggedByCurrentUser__c` (`nebula-logger/core/main/log-management/objects/LogEntry__c/fields/WasLoggedByCurrentUser__c.field-meta.xml:1`): 1 parsed, 11 textual
- `Logger.CURRENT_VERSION_NUMBER` (`nebula-logger/core/main/logger-engine/classes/Logger.cls:17`): 2 parsed, 5 textual
- `LoggerEndUser` (`nebula-logger/core/main/log-management/permissionsets/LoggerEndUser.permissionset-meta.xml:1`): 0 parsed, 3 textual
- `logger` (`nebula-logger/core/main/logger-engine/lwc/logger/logger.js:1`): 8 parsed, 3718 textual

<details>
<summary>Possible tests (70) — not selected</summary>

- `CMDT_LogEntryDataMaskRule_Tests` (`nebula-logger/extra-tests/bundled-custom-metadata-type-records/CMDT_LogEntryDataMaskRule_Tests.cls:13`): `lwc_textual`, `textual_path`; seed `logger`
- `CMDT_LogStatus_Tests` (`nebula-logger/extra-tests/bundled-custom-metadata-type-records/CMDT_LogStatus_Tests.cls:13`): `lwc_textual`; seed `logger`
- `CMDT_LoggerParameter_Tests` (`nebula-logger/extra-tests/bundled-custom-metadata-type-records/CMDT_LoggerParameter_Tests.cls:13`): `lwc_textual`, `textual_path`; seed `logger`
- `CMDT_LoggerSObjectHandler_Tests` (`nebula-logger/extra-tests/bundled-custom-metadata-type-records/CMDT_LoggerSObjectHandler_Tests.cls:13`): `lwc_textual`; seed `logger`
- `CallableLogger_Tests` (`nebula-logger/core/tests/logger-engine/classes/CallableLogger_Tests.cls:6`): `lwc_textual`, `textual_path`; seed `logger`
- `ComponentLogger_Tests` (`nebula-logger/core/tests/logger-engine/classes/ComponentLogger_Tests.cls:6`): `lwc_textual`, `textual_path`; seed `logger`
- `FlowCollectionLogEntry_Tests` (`nebula-logger/core/tests/logger-engine/classes/FlowCollectionLogEntry_Tests.cls:6`): `lwc_textual`, `textual_path`; seed `logger`
- `FlowLogEntry_Tests` (`nebula-logger/core/tests/logger-engine/classes/FlowLogEntry_Tests.cls:6`): `lwc_textual`, `textual_path`; seed `logger`
- `FlowLogger_Tests` (`nebula-logger/core/tests/logger-engine/classes/FlowLogger_Tests.cls:6`): `lwc_textual`, `textual_path`; seed `logger`
- `FlowRecordLogEntry_Tests` (`nebula-logger/core/tests/logger-engine/classes/FlowRecordLogEntry_Tests.cls:6`): `lwc_textual`, `textual_path`; seed `logger`
- `LogBatchApexErrorEventHandler_Tests` (`nebula-logger/plugins/async-failure-additions/plugin/classes/LogBatchApexErrorEventHandler_Tests.cls:5`): `lwc_textual`, `textual_path`; seed `logger`
- `LogBatchPurgeController_Tests` (`nebula-logger/core/tests/log-management/classes/LogBatchPurgeController_Tests.cls:5`): `lwc_textual`, `textual_path`; seed `LoggerEndUser`
- `LogBatchPurgeScheduler_Tests` (`nebula-logger/core/tests/log-management/classes/LogBatchPurgeScheduler_Tests.cls:6`): `lwc_textual`, `textual_path`; seed `LoggerEndUser`
- `LogBatchPurger_Tests` (`nebula-logger/core/tests/log-management/classes/LogBatchPurger_Tests.cls:6`): `lwc_textual`, `textual_path`; seed `logger`
- `LogBatchPurger_Tests_Database` (`nebula-logger/extra-tests/integration-tests/classes/LogBatchPurger_Tests_Database.cls:6`): `lwc_textual`, `textual_path`; seed `logger`
- `LogBatchPurger_Tests_Flow` (`nebula-logger/extra-tests/plugins/classes/LogBatchPurger_Tests_Flow.cls:6`): `lwc_textual`, `textual_path`; seed `logger`
- `LogEntryArchiveBuilder_Tests` (`nebula-logger/plugins/big-object-archiving/plugin/classes/LogEntryArchiveBuilder_Tests.cls:6`): `lwc_textual`; seed `logger`
- `LogEntryArchiveController_Tests` (`nebula-logger/plugins/big-object-archiving/plugin/classes/LogEntryArchiveController_Tests.cls:6`): `lwc_textual`; seed `logger`
- `LogEntryArchivePlugin_Tests` (`nebula-logger/plugins/big-object-archiving/plugin/classes/LogEntryArchivePlugin_Tests.cls:6`): `lwc_textual`, `textual_path`; seed `logger`
- `LogEntryEventBuilder_Tests` (`nebula-logger/core/tests/logger-engine/classes/LogEntryEventBuilder_Tests.cls:6`): `lwc_textual`, `textual_path`; seed `logger`
- `LogEntryEventBuilder_Tests_Network` (`nebula-logger/extra-tests/integration-tests/classes/LogEntryEventBuilder_Tests_Network.cls:6`): `lwc_textual`, `textual_path`; seed `logger`
- `LogEntryEventBuilder_Tests_Security` (`nebula-logger/extra-tests/integration-tests/classes/LogEntryEventBuilder_Tests_Security.cls:6`): `lwc_textual`, `textual_path`; seed `logger`
- `LogEntryEventHandler_Tests` (`nebula-logger/core/tests/log-management/classes/LogEntryEventHandler_Tests.cls:6`): `lwc_textual`, `textual_path`; seed `logger`
- `LogEntryEventHandler_Tests_FieldMappings` (`nebula-logger/extra-tests/integration-tests/classes/LogEntryEventHandler_Tests_FieldMappings.cls:6`): `lwc_textual`, `textual_path`; seed `LoggerEndUser`
- `LogEntryEventStreamController_Tests` (`nebula-logger/core/tests/log-management/classes/LogEntryEventStreamController_Tests.cls:6`): `lwc_textual`, `textual_path`; seed `LoggerEndUser`
- `LogEntryFieldSetPicklist_Tests` (`nebula-logger/core/tests/log-management/classes/LogEntryFieldSetPicklist_Tests.cls:6`): `lwc_textual`, `textual_path`; seed `logger`
- `LogEntryHandler_Tests` (`nebula-logger/core/tests/log-management/classes/LogEntryHandler_Tests.cls:6`): `lwc_textual`, `textual_path`; seed `logger`
- `LogEntryHandler_Tests_EmailMessage` (`nebula-logger/extra-tests/integration-tests/classes/LogEntryHandler_Tests_EmailMessage.cls:6`): `lwc_textual`, `textual_path`; seed `LoggerEndUser`
- `LogEntryHandler_Tests_Flow` (`nebula-logger/extra-tests/integration-tests/classes/LogEntryHandler_Tests_Flow.cls:6`): `lwc_textual`, `textual_path`; seed `LoggerEndUser`
- `LogEntryMetadataViewerController_Tests` (`nebula-logger/core/tests/log-management/classes/LogEntryMetadataViewerController_Tests.cls:1`): `lwc_textual`; seed `logger`
- `LogEntryMetadataViwrCtlr_Tests_Security` (`nebula-logger/extra-tests/integration-tests/classes/LogEntryMetadataViwrCtlr_Tests_Security.cls:9`): `lwc_textual`, `textual_path`; seed `LoggerEndUser`
- `LogEntryTagHandler_Tests` (`nebula-logger/core/tests/log-management/classes/LogEntryTagHandler_Tests.cls:6`): `lwc_textual`, `textual_path`; seed `LoggerEndUser`
- `LogFinalizer_Tests` (`nebula-logger/plugins/async-failure-additions/plugin/classes/LogFinalizer_Tests.cls:5`): `lwc_textual`; seed `logger`
- `LogFlowExecutionErrorEventHandler_Tests` (`nebula-logger/plugins/async-failure-additions/plugin/classes/LogFlowExecutionErrorEventHandler_Tests.cls:5`): `lwc_textual`; seed `logger`

**36 more not shown**; see impact.json artifact.

</details>

<details>
<summary>Direct referrers (parsed first)</summary>

- `LogEntry__c.WasLoggedByCurrentUser__c` ← parsed `permission_set` `LoggerEndUser` (`nebula-logger/core/main/log-management/permissionsets/LoggerEndUser.permissionset-meta.xml`)
- `Logger.CURRENT_VERSION_NUMBER` ← parsed `method` `Logger.getVersionNumber()` (`nebula-logger/core/main/logger-engine/classes/Logger.cls`)
- `Logger.CURRENT_VERSION_NUMBER` ← parsed `method` `Logger_Tests.it_should_return_version_number()` (`nebula-logger/core/tests/logger-engine/classes/Logger_Tests.cls`)
- `logger` ← parsed `lwc` `logger` (`nebula-logger/core/main/logger-engine/lwc/logger/logger.js`)
- `logger` ← parsed `lwc` `loggerLWCCreateLoggerImportDemo` (`nebula-logger/recipes/lwc/loggerLWCCreateLoggerImportDemo/loggerLWCCreateLoggerImportDemo.js`)
- `logger` ← parsed `lwc` `loggerLWCEmbedDemo` (`nebula-logger/recipes/lwc/loggerLWCEmbedDemo/loggerLWCEmbedDemo.js`)
- `logger` ← parsed `lwc` `loggerLWCGetLoggerImportDemo` (`nebula-logger/recipes/lwc/loggerLWCGetLoggerImportDemo/loggerLWCGetLoggerImportDemo.js`)
- `logger` ← parsed `lwc` `loggerWrapper` (`nebula-logger/recipes/lwc/loggerWrapper/loggerWrapper.js`)
- `LogEntry__c.WasLoggedByCurrentUser__c` ← textual `file` `nebula-logger/core/main/log-management/objects/LogEntry__c/listViews/MyGeneratedLogEntries.listView-meta.xml` (`nebula-logger/core/main/log-management/objects/LogEntry__c/listViews/MyGeneratedLogEntries.listView-meta.xml`)
- `LogEntry__c.WasLoggedByCurrentUser__c` ← textual `file` `nebula-logger/core/main/log-management/objects/Log__c/listViews/MyGeneratedLogs.listView-meta.xml` (`nebula-logger/core/main/log-management/objects/Log__c/listViews/MyGeneratedLogs.listView-meta.xml`)
- `LogEntry__c.WasLoggedByCurrentUser__c` ← textual `lwc` `logBatchPurge` (`nebula-logger/core/main/log-management/lwc/logBatchPurge/logBatchPurge.js`)
- `LogEntry__c.WasLoggedByCurrentUser__c` ← textual `lwc` `logViewer` (`nebula-logger/core/main/log-management/lwc/logViewer/logViewer.js`)
- `LogEntry__c.WasLoggedByCurrentUser__c` ← textual `object_field` `LogEntry__c.WasLoggedByCurrentUser__c` (`nebula-logger/core/main/log-management/objects/LogEntry__c/fields/WasLoggedByCurrentUser__c.field-meta.xml`)
- `LogEntry__c.WasLoggedByCurrentUser__c` ← textual `object_field` `Log__c.WasLoggedByCurrentUser__c` (`nebula-logger/core/main/log-management/objects/Log__c/fields/WasLoggedByCurrentUser__c.field-meta.xml`)
- `LogEntry__c.WasLoggedByCurrentUser__c` ← textual `permission_set` `LoggerEndUser` (`nebula-logger/core/main/log-management/permissionsets/LoggerEndUser.permissionset-meta.xml`)
- `Logger.CURRENT_VERSION_NUMBER` ← textual `field` `Logger.CURRENT_VERSION_NUMBER` (`nebula-logger/core/main/logger-engine/classes/Logger.cls`)
- `Logger.CURRENT_VERSION_NUMBER` ← textual `lwc` `logger` (`nebula-logger/core/main/logger-engine/lwc/logger/logger.js`)
- `Logger.CURRENT_VERSION_NUMBER` ← textual `method` `Logger.getVersionNumber()` (`nebula-logger/core/main/logger-engine/classes/Logger.cls`)
- `Logger.CURRENT_VERSION_NUMBER` ← textual `method` `Logger_Tests.it_should_return_version_number()` (`nebula-logger/core/tests/logger-engine/classes/Logger_Tests.cls`)
- `LoggerEndUser` ← textual `class` `LoggerTestConfigurator` (`nebula-logger/core/tests/log-management/utilities/LoggerTestConfigurator.cls`)
- `LoggerEndUser` ← textual `method` `LoggerTestConfigurator.assignEndUserPermissionSet(id)` (`nebula-logger/core/tests/log-management/utilities/LoggerTestConfigurator.cls`)
- `LoggerEndUser` ← textual `method` `LoggerTestConfigurator.queryPermissionSets()` (`nebula-logger/core/tests/log-management/utilities/LoggerTestConfigurator.cls`)
- `logger` ← textual `class` `CallableLogger` (`nebula-logger/core/main/logger-engine/classes/CallableLogger.cls`)
- `logger` ← textual `class` `ComponentLogger` (`nebula-logger/core/main/logger-engine/classes/ComponentLogger.cls`)
- `logger` ← textual `class` `ComponentLogger.ComponentLogEntry` (`nebula-logger/core/main/logger-engine/classes/ComponentLogger.cls`)
- `logger` ← textual `class` `ComponentLogger.ComponentLoggerSettings` (`nebula-logger/core/main/logger-engine/classes/ComponentLogger.cls`)
- `logger` ← textual `class` `FlowCollectionLogEntry` (`nebula-logger/core/main/logger-engine/classes/FlowCollectionLogEntry.cls`)
- `logger` ← textual `class` `FlowLogEntry` (`nebula-logger/core/main/logger-engine/classes/FlowLogEntry.cls`)
- `logger` ← textual `class` `FlowLogger` (`nebula-logger/core/main/logger-engine/classes/FlowLogger.cls`)
- `logger` ← textual `class` `FlowLogger.LogEntry` (`nebula-logger/core/main/logger-engine/classes/FlowLogger.cls`)
- `logger` ← textual `class` `FlowRecordLogEntry` (`nebula-logger/core/main/logger-engine/classes/FlowRecordLogEntry.cls`)
- `logger` ← textual `class` `LogBatchPurger` (`nebula-logger/core/main/log-management/classes/LogBatchPurger.cls`)
- `logger` ← textual `class` `LogEntryArchivePlugin` (`nebula-logger/plugins/big-object-archiving/plugin/classes/LogEntryArchivePlugin.cls`)
- `logger` ← textual `class` `LogEntryEventBuilder` (`nebula-logger/core/main/logger-engine/classes/LogEntryEventBuilder.cls`)
- `logger` ← textual `class` `LogRetentionRulesPlugin` (`nebula-logger/plugins/log-retention-rules/plugin/classes/LogRetentionRulesPlugin.cls`)
- `logger` ← textual `class` `Logger` (`nebula-logger/core/main/logger-engine/classes/Logger.cls`)
- `logger` ← textual `class` `LoggerEmailSender_Tests` (`nebula-logger/core/tests/log-management/classes/LoggerEmailSender_Tests.cls`)
- `logger` ← textual `class` `LoggerEngineDataSelector` (`nebula-logger/core/main/logger-engine/classes/LoggerEngineDataSelector.cls`)
- `logger` ← textual `class` `LoggerHomeHeaderController` (`nebula-logger/core/main/log-management/classes/LoggerHomeHeaderController.cls`)

**1110 more not shown**; see impact.json artifact.

</details>

Full evidence: impact.json artifact.
