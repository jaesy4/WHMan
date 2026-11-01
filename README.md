# Warehouse In/Out Inventory (Rust)

An inventory app for the warehouse man: **Stock In, Stock Out and balances**. There are no prices or Total Value.
It runs as a **Windows `.exe`** and an **Android `.apk`** from one Rust codebase. The phone and the PC sync over local Wi-Fi, with no internet or cloud server.

```
inventory_system/
├── Cargo.toml                  Workspace
├── Trunk.toml                  Builds the UI to WebAssembly
├── app-icon.png                Source icon (1024×1024)
├── crates/
│   ├── inventory_types/        Shared data types (used by core + UI)
│   └── inventory_core/         The engine: no UI, fully unit-tested
│       ├── src/db.rs           SQLite schema, migrations, device id, sync clock
│       ├── src/items.rs        Item master list, inventory balances
│       ├── src/movements.rs    Stock In / Stock Out, void
│       ├── src/reports.rs      In/Out summary, stock card, dashboard
│       ├── src/excel.rs        Excel import (preview → confirm) and export
│       ├── src/pdf/            PDF reports and IN/OUT slips (built-in engine)
│       ├── src/sync/           LAN sync: server (PC), client (phone), discovery
│       └── tests/              Integration tests
├── src-tauri/                  Tauri v2 app (Windows + Android)
│   ├── src/lib.rs              Commands the UI calls
│   ├── tauri.conf.json
│   ├── capabilities/           Permissions (dialogs, files, QR camera)
│   └── icons/
└── ui/                         Leptos screens (Rust → WebAssembly)
    ├── src/main.rs             Shell, navigation, toasts
    ├── src/pages/              Home, Inventory, Stock In/Out, Transactions,
    │                           Stock Card, Reports, Import, Sync, Settings
    └── styles/app.css
```

## How it works

| Topic | Design |
|---|---|
| **Stock** | Never stored as one number. Every IN and OUT is its own record, and the balance is their sum. Records from the phone and the PC merge without overwriting each other. |
| **Quantities** | Stored as integer thousandths (2.5 kg is stored as 2500), so totals never drift from rounding. |
| **Rules** | A Stock Out can't exceed the balance. Item codes must be unique. Items with stock can't be deleted. Wrong entries are **voided**: kept in the history marked VOID, but no longer counted. |
| **IDs** | UUIDs, so both devices can safely create items while offline. |
| **Excel import** | Columns are recognised by header name, in any order. You see a preview with errors listed by row before anything is saved, and the save is all-or-nothing. |
| **PDF** | A small built-in engine with no external library. Reports include signature lines (Prepared / Checked / Noted by). |
| **Sync** | The PC is the hub and runs an HTTP server inside the app. The phone pairs by scanning a QR code. Every request is signed with HMAC-SHA256, so the secret is never sent over the network. Sync is incremental: only changes since the last sync are sent. If two devices edit the same item, the latest edit wins. A void always wins. |

Ports used on the PC: **TCP 47810** (sync) and **UDP 47811** (finding the PC on the network).

---

## Build on GitHub (no local setup)

`.github/workflows/build.yml` compiles everything on GitHub's servers.

1. Create a repository on GitHub and upload this whole folder, including the hidden `.github` folder.
2. Open the **Actions** tab. Each push starts a build that takes about 15–25 minutes:
   - **Test core** runs the engine tests.
   - **Windows (.exe)** builds the installer.
   - **Android (.apk)** builds and signs the APK.
3. Open the finished run. Under **Artifacts**, download `windows-installer` and `android-apk`.
4. Optional: push a tag such as `v0.1.0`. The files are then also published on the repository's **Releases** page.

**Keeping the same Android signing key.** Without the secrets below, each build is signed with a new temporary key, so phones must uninstall the old app before installing a new build. To keep one key, create a keystore once (see section 5) and add three repository secrets under *Settings → Secrets and variables → Actions*:

- `ANDROID_KEYSTORE_BASE64`: the output of `[Convert]::ToBase64String([IO.File]::ReadAllBytes("$HOME\warehouse-upload.jks"))`
- `ANDROID_KEYSTORE_PASSWORD`
- `ANDROID_KEY_ALIAS`: `upload`

If a build fails, open the red job, copy the error lines, and send them over.

---

## 1. Install the tools (Windows PC, one time)

1. **Rust**: install from https://rustup.rs. When asked, also install the **Visual Studio C++ Build Tools** ("Desktop development with C++").
2. **WebView2**: already included in Windows 10 and 11.
3. In **PowerShell**:

```powershell
rustup target add wasm32-unknown-unknown
cargo install trunk --locked
cargo install tauri-cli --version "^2" --locked
```

## 2. Test the engine

```powershell
cd inventory_system
cargo test -p inventory_core --all-features
```

This runs the tests for stock rules, reports, the Excel round trip, PDF output, offline merging, and a real pair-and-sync over HTTP.

## 3. Run the app in development

```powershell
cargo tauri dev
```

## 4. Build the Windows installer (`.exe`)

```powershell
cargo tauri build
```

Output:

- `target\release\bundle\nsis\Warehouse Inventory_0.1.0_x64-setup.exe`: the installer to give users
- `target\release\bundle\msi\Warehouse Inventory_0.1.0_x64_en-US.msi`: the same app as an MSI
- `target\release\warehouse-inventory.exe`: the app itself

The first time the app runs, Windows Firewall asks for network access. Choose **Private networks**. The phone sync needs this.

## 5. Build the Android app (`.apk`)

**One-time setup:**

1. Install **Android Studio**. In *SDK Manager*, install:
   - Android SDK Platform (API 34 or newer)
   - Android SDK Platform-Tools
   - Android SDK Build-Tools
   - Android SDK Command-line Tools
   - **NDK (Side by side)**
2. Set the environment variables. Adjust the NDK version folder to match what you installed:

```powershell
[System.Environment]::SetEnvironmentVariable("JAVA_HOME", "C:\Program Files\Android\Android Studio\jbr", "User")
[System.Environment]::SetEnvironmentVariable("ANDROID_HOME", "$env:LocalAppData\Android\Sdk", "User")
$ndk = Get-ChildItem "$env:LocalAppData\Android\Sdk\ndk" | Select-Object -Last 1
[System.Environment]::SetEnvironmentVariable("NDK_HOME", $ndk.FullName, "User")
```

3. Close and reopen PowerShell. Then run:

```powershell
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
cargo tauri icon app-icon.png      # generates all desktop + Android icons
cargo tauri android init           # creates src-tauri\gen\android (commit this folder)
```

**Build:**

```powershell
cargo tauri android build --apk
# smaller APK for modern phones only:
cargo tauri android build --apk --target aarch64
```

Output: `src-tauri\gen\android\app\build\outputs\apk\universal\release\app-universal-release-unsigned.apk`

**Signing (required to install on phones):**

1. Create a keystore once, and keep it safe:

```powershell
keytool -genkey -v -keystore $HOME\warehouse-upload.jks -keyalg RSA -keysize 2048 -validity 10000 -alias upload
```

2. Create `src-tauri\gen\android\keystore.properties`:

```
password=YOUR_PASSWORD
keyAlias=upload
storeFile=C:\\Users\\YOU\\warehouse-upload.jks
```

3. In `src-tauri\gen\android\app\build.gradle.kts`:
   - At the top, add `import java.io.FileInputStream`.
   - Inside `android { ... }`, before `buildTypes`, add:

```kotlin
signingConfigs {
    create("release") {
        val props = java.util.Properties()
        props.load(FileInputStream(rootProject.file("keystore.properties")))
        keyAlias = props["keyAlias"] as String
        keyPassword = props["password"] as String
        storeFile = file(props["storeFile"] as String)
        storePassword = props["password"] as String
    }
}
```

   - Inside `buildTypes { getByName("release") { ... } }`, add `signingConfig = signingConfigs.getByName("release")`.

4. Build again. The output is now `app-universal-release.apk`. Install it with `adb install` or by copying it to the phone.

To run the app on a phone connected by USB during development: `cargo tauri android dev`.

---

## 6. Using the app

**First time (on the PC):**

1. Go to **Settings** and enter the warehouse name (printed on reports) and your name (used as "Recorded by").
2. Go to **Import Excel** and click **Download item template**. Fill in your items and their beginning balances, choose the file, check the preview, then click **Import**.
   - You can also add items one by one in **Inventory**.

**Daily work:**

- **Stock In** (deliveries): item, quantity, date, DR number, received from.
- **Stock Out** (issuance): item, quantity, date, request slip number, issued to.
- After saving a record, you can print a **Receiving Slip** or **Issue Slip** for signatures.
- **Transactions**: search records, reprint slips, and **Void** wrong entries (a reason is required).
- **Stock Card**: one item's full history with the running balance.
- **Reports** (PDF or Excel):
  - Inventory
  - Low Stock
  - In/Out Summary (Beginning, In, Out, Ending) for any period
  - Stock In report
  - Stock Out report

**Pairing a phone:**

1. On the PC, go to **Sync** and click **Pair a phone**. A QR code appears; it's valid for 5 minutes and works once.
2. On the phone, go to **Sync** and tap **Scan PC QR code**.
3. From then on, tap **Sync now** on the phone whenever it's on the warehouse Wi-Fi with the PC app open.
   - Records made offline on either device are merged.
   - If the PC gets a new IP address, the phone finds it automatically.

**Backup:** copy `inventory.db` from `%APPDATA%\com.warehouse.inventory\`.

---

## Notes

- I wrote this code in an environment that could not download crates, so only these parts were compiled and tested there: the PDF engine, LAN discovery, and all SQL. The rest is written against the versions pinned in the `Cargo.toml` files. If `cargo` reports a compile error (for example, a small API change in a newer crate version), send the error message and it can be fixed quickly.
- The phone syncs over plain HTTP on the local network, with every request signed by HMAC. The pairing secret travels only inside the QR code. Use it on a trusted Wi-Fi, such as the warehouse network.
