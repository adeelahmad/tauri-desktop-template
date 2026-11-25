# Brand

The name, icon and colours of **{{app_name}}**, and where each one lives. Change them here
first, then in the files listed.

## Name

| What | Value | Where |
|---|---|---|
| Name shown to users | {{app_name}} | `productName` and the window `title` in `src-tauri/tauri.conf.json`, the page title in {% if frontend == "react" %}`ui-react/index.html`{% else %}`ui/index.html`{% endif %} |
| Crate and executable | `{{project-name}}` | `src-tauri/Cargo.toml` |
| Bundle identifier | `{{identifier}}` | `identifier` in `src-tauri/tauri.conf.json` (changing it moves the settings and data folders) |
| One-line description | {{description}} | `shortDescription` in `src-tauri/tauri.conf.json`, `description` in `src-tauri/Cargo.toml` |
| Repository | `{{github_repo}}` | README badges{% if updater %}, the updater endpoint{% endif %}{% if install_scripts %}, `scripts/`{% endif %} |

## Icon

The source is `src-tauri/icons/source-1024.png`: a square PNG, 1024 × 1024
(a transparent background works best). Every other file in `src-tauri/icons/` is generated from it.

To change the icon, replace that file and regenerate the rest from the app's root folder:

```sh
cargo tauri icon src-tauri/icons/source-1024.png
```

This writes `32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.icns` (macOS), `icon.ico`
(Windows), `icon.png` and the Windows Store sizes into `src-tauri/icons/`{% if mobile %}, and
the Android and iOS icons once those projects exist (`cargo tauri android init` / `ios init`){% endif %}.
The bundle uses the files listed under `bundle.icon` in `src-tauri/tauri.conf.json`.
Install the Tauri CLI with `cargo install tauri-cli --version '=3.0.0-alpha.6' --locked` if
`cargo tauri` is missing (`kitchen doctor` checks it).

## Colours

| Token | Value | Use |
|---|---|---|
| `--accent` | `#3b6eea` | buttons, links, highlights |
| `--muted` | `#777` | secondary text |
| `--line` | `#8884` | borders and dividers |
| error | `#d33` | error messages |

They are CSS custom properties at the top of {% if frontend == "react" %}`ui-react/src/style.css`{% else %}`ui/style.css`{% endif %}.
The UI follows the system's light or dark mode (`color-scheme: light dark`), so check both
after a change. Keep text on `--accent` readable (white text needs a contrast ratio of 4.5:1).
