# Main-window background sources

The main-window background is a saved source choice (solid color, copied image, recursive slideshow, current track artwork, or Windows wallpaper image). For non-solid modes, blur, image opacity, and zoom remain separate saved appearance settings. Zoom ranges from 0% to 10%, defaults to 3%, and applies to the main-window backdrop; the Now Playing backdrop keeps its own 3% baseline. System wallpaper is read as an image instead of making the application window transparent, because transparency would reveal whichever window is underneath and does not guarantee that the desktop wallpaper is visible.

## Android (2026-10-17)

- **Single image**: the system document picker (`ACTION_OPEN_DOCUMENT`, PNG/JPEG/WebP/GIF/BMP) is used, and the picked bytes are copied into `UserData/background` through a size-capped cache lease (32 MiB), exactly like the desktop copy. No persisted URI grant is kept, so the background survives the source file being moved or the grant being revoked.
- **Slideshow**: the source is a SAF document tree (persisted read grant, stored as `StoredPath::Uri`). The native plugin lists images recursively (hidden entries skipped, same formats as desktop, sorted case-insensitively by relative path, capped at 10,000). The shared `MainBackgroundRuntime` drives order, interval, and random cycles with an injected scanner, using the tree URI as the source key; each image is read through a short-lived cache lease. The desktop has no multi-image selection, so Android does not add one.
- **System wallpaper**: Android 13+ (targetSdk 33+) throws `SecurityException` from `WallpaperManager.getDrawable`/`getWallpaperFile`/`peekDrawable`; on Android 14+ only `MANAGE_EXTERNAL_STORAGE` or the privileged `READ_WALLPAPER_INTERNAL` allow it, and Google treats this as intended behaviour. A live wallpaper has no bitmap at all. We reject `MANAGE_EXTERNAL_STORAGE`: it means all-files access, Play policy restricts it, and asking for it only to read the wallpaper is out of proportion. Instead, the activity window sets `FLAG_SHOW_WALLPAPER` with a transparent window and WebView background, so the compositor draws the current wallpaper (static or live) behind the UI without any permission, and wallpaper changes apply at once. The web layer draws `--page` at `1 − opacity` over it. Blur uses `Window.setBackgroundBlurRadius` only when `WindowManager.isCrossWindowBlurEnabled`. Unlike Windows, Android does not read the image, so it cannot sample its average colour for automatic contrast.

### Considered options (wallpaper)

1. `FLAG_SHOW_WALLPAPER` (chosen): no permission, works with live wallpapers, no pixels are available to the app.
2. `MANAGE_EXTERNAL_STORAGE` + `getDrawable`: provides the pixels, but needs an all-files special permission and a policy declaration. Rejected.
3. Ask the user to pick the wallpaper image file manually: this is already possible through Single image.
