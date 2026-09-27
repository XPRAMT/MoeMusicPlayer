# Windows Search / SystemIndex 信任界線

目前 Windows adapter 對設定為 SystemIndex 的 root 會檢查 `WSearch` 服務、SystemIndex 的 crawl-scope 規則與全域 catalog 狀態，並將診斷附在來源掃描錯誤中。即使服務正在執行、root 被 scope 規則包含、沒有子 scope 規則且 catalog 顯示 Idle，曲目列舉與刪除對帳仍只使用完整的檔案系統掃描。

評估過的作法有兩種：直接把 Windows Search 查詢列當成完整 seen-set，或將 Search 狀態只用於診斷並以檔案系統走訪建立 seen-set。前者在現有公開 API 下無法證明缺席代表檔案已刪除；採用後者，讓穩定性與刪除安全依既有 filesystem scanner 保證。診斷不會建立、修改或啟動 Windows Search 服務，也不會改寫其 scope 規則。

Windows Search 的公開 API 無法證明指定資料夾的查詢結果完整且即時：

- `IncludedInCrawlScope` 僅回報 URL 是否符合 Crawl Scope Manager 規則。Microsoft 文件明確指出，FANCI（File Attribute Not Content Indexed）等其他條件仍可阻止項目被爬取，即使 scope 規則包含該路徑。
- `HasChildScopeRule` 能指出 root 下有子規則，但沒有子規則仍不代表每個檔案都被索引。
- `CATALOG_STATUS_IDLE` 表示目前沒有待處理的索引工作；它不是指定 root 的覆蓋率、列舉完成或檔案變更 watermark。
- Windows Search 的索引流程依賴通知與爬取；官方文件指出通知來源失效時，新檔或變更可能不會進入索引，直到後續爬取。
- `SCOPE` 查詢只是限制索引中要回傳的項目。空結果只代表查詢時索引沒有回傳項目，不能證明檔案系統中沒有音訊檔。

因此 adapter 不把 SystemIndex 結果用作 seen-set，也不允許它觸發刪除。查詢、scope 或服務狀態不可用時照樣走檔案系統 fallback；若 fallback 無法完整走訪，Core 會回報 `Incomplete` 或 `Unavailable`，資料庫不會刪除該 root 的既有來源映射。要安全切換到 SystemIndex 做完整列舉，仍需要可驗證的逐項涵蓋與新鮮度保證；目前公開狀態 API 未提供這種保證。

使用的 binding 依賴是 `windows 0.62.2`，僅在 Windows target 啟用，並限開 Foundation、COM、Search、Services features。其 crate manifest 標示 MIT OR Apache-2.0、最低 Rust 1.82；Microsoft `windows-rs` 官方倉庫仍有持續發布與維護。它提供本 adapter 所需的型別化 COM/Search/SCM API，避免手寫 COM vtable。未引用或複製其他播放器程式碼。

官方文件：

- [Managing scope rules](https://learn.microsoft.com/en-us/windows/win32/search/-search-3x-wds-extidx-csm-scoperules)
- [IncludedInCrawlScope](https://learn.microsoft.com/en-us/windows/win32/api/searchapi/nf-searchapi-isearchcrawlscopemanager-includedincrawlscope)
- [CatalogStatus](https://learn.microsoft.com/en-us/windows/win32/api/searchapi/ne-searchapi-catalogstatus)
- [Indexing process in Windows Search](https://learn.microsoft.com/en-us/windows/win32/search/-search-indexing-process-overview)
- [SCOPE and DIRECTORY predicates](https://learn.microsoft.com/en-us/windows/win32/search/-search-sql-folderdepth)
- [windows 0.62.2 manifest](https://docs.rs/crate/windows/0.62.2/source/Cargo.toml.orig)
- [Microsoft windows-rs releases](https://github.com/microsoft/windows-rs/releases)
