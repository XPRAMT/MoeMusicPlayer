# Android 平台整合：本機來源、播放服務、權限說明面板

Status: 已採納

## Context

Android 的資料夾選擇、播放服務生命週期、通知、回前景空白畫面的修正與權限說明面板原本只記在 AGENTS.md（另見 ADR 0006、0014）。此 ADR 記錄其目前設計。

## 目前設計

本機來源（ADR 0014）：資料夾來源用 `AndroidFilesystem`／`fs_scan`；SAF 資料夾選擇器只回傳實體位置、不保留 tree grant。已有所有檔案存取權時改用 App 自己的資料夾瀏覽器（系統選擇器不允許選儲存根目錄）：`android_folder_browser::pick_directory`（新增來源、排除資料夾、我的最愛資料夾、幻燈片資料夾共用）發 `android-folder-picker-request` 並等待（最長 15 分鐘），`FolderBrowserDialog.svelte` 以 `android_list_directories` 列出 `/storage` 下的子資料夾，結果經 `android_folder_picker_result`；返回鍵＝取消。排除資料夾用實際路徑存在 `excluded_folders`（與 Windows 同欄），`fs_scan::scan_directory`（`with_adapter_excluding`）略過；排除不能包含來源根目錄，來源也不能加在排除路徑內。舊 `AndroidMediaStore`／`AndroidSaf` 來源不再自動轉換（保持不可用並保留對應，先以資料夾來源重新加入並同步，再手動移除）。啟動時只清除 app data 根目錄直層 `pre-path-migration-<數字>` 的舊快照目錄。手機實測待做：資料夾瀏覽器、「在資料夾中開啟」、排除資料夾、本機歌詞候選、APK 更新。

播放服務與通知：播放在 `plugins/tauri-plugin-media-index` 的 Media3 `PlaybackService`（MediaSessionService＋ExoPlayer，Rust 經 JNI 輪詢指令，佇列在 Rust；`QueueAwarePlayer` 依 `set_capabilities` 開放上一首／下一首，按鈕以 `transport` 事件回報）；音訊生命週期不得依賴 Activity 存活，`MainActivity.onStart` 的 `ensurePlaybackService` 每次回前景 `startService`。服務以 startService 啟動，`onCreate` 必須 `addSession(session)` 才有 MediaStyle 通知（通知欄＋鎖定畫面），播放時為 `mediaPlayback` 前景服務；`onDestroy` 先 `removeSession`，`onTaskRemoved` 未播放時 `stopSelf`。載入指令帶標題／演出者／專輯與縮小封面（`setArtworkData`）。Android 13+ 的 `POST_NOTIFICATIONS` 依 `NotificationPermissionPolicy` 每行程最多詢問一次。回前景畫面只剩底色 #141218 的修正：`MainActivity.onWindowFocusChanged(true)` 立刻對 WebView 做 `onPause`＋`onResume`；若再出現，以 `WebView.setWebContentsDebuggingEnabled(true)`、`adb forward tcp:19333 localabstract:webview_devtools_remote_<pid>`（9333 在 Windows 保留埠內）抓 `android_webview,cc,viz,gpu` Tracing。

權限說明面板（沿用 `.update-dialog`／`.floating-surface` 外觀）：「所有檔案存取權」未授權時每行程自動彈出一次（`App.svelte` 的 `allFilesAccessAutoPrompted`、`allFilesPromptOpen`；「前往授權」才開系統頁，「取消」／Esc／返回只關閉）。系統桌布：HyperOS 讀桌布還需 `READ_MEDIA_IMAGES`（外掛 manifest 宣告；Kotlin `mediaImages`、`mediaImagesPermissionStatus`／`requestMediaImagesPermission`；App 命令 `android_system_wallpaper_reason`／`android_request_media_images_permission`／`android_media_images_permission_status`），只有使用者剛選「系統桌布」且讀取因此被拒時才彈出說明面板，授權後立即重讀。區域網路：targetSdk 37，manifest 宣告 `ACCESS_LOCAL_NETWORK`；Android 17 未授權時點「新增遠端伺服器」先顯示說明面板，「前往授權」由 `MainActivity` 經 `MoeAndroidInsets` bridge 請求，獲准後開表單；測試帳號或瀏覽遠端前也先請求，拒絕時不呼叫遠端 IPC。API 37 系統提示的實機驗收待具備該版本的裝置。多個 SMB App 同時連不上時，NAS 445 埠、防火牆、Wi-Fi／VLAN 隔離也要列入調查。
