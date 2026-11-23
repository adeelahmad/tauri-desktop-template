# Security policy

## Supported versions

Only the newest release gets fixes. Installed apps update themselves when the updater is on.

## Reporting a vulnerability

Please do not open a public issue. Use
[a private security advisory](https://github.com/{{github_repo}}/security/advisories/new)
with steps to reproduce and the version you tested. Expect a first answer within a week.

## What the app does locally

- All data (documents, vectors, the embedded database) stays in the user's app-data folder.
- The embedded Postgres listens on localhost only, with a random password stored with
  mode 0600.
- The worker socket is per user (named pipe on Windows, abstract or temp-folder socket elsewhere).
- Update bundles are verified against the public key in `tauri.conf.json` before install.
