## Install {{app_name}}

These builds are not signed with an Apple Developer ID or a Windows code-signing
certificate, so each system asks once.

**macOS (Apple Silicon)**
{% if install_scripts %}
- Easiest, no prompts: open Terminal and paste
  `curl -fsSL https://raw.githubusercontent.com/{{github_repo}}/main/scripts/install.sh | sh`
{% endif -%}
{% if macos_pkg %}- Or download the `.pkg`, right-click it, choose **Open**, then **Open** again. The app
  lands in Applications and opens normally from then on.
{% endif -%}
- Or download the `.dmg`, drag the app to Applications, then right-click it and choose
  **Open** the first time (or System Settings > Privacy & Security > **Open Anyway**).

**Windows**
{% if install_scripts %}
- In PowerShell: `irm https://raw.githubusercontent.com/{{github_repo}}/main/scripts/install.ps1 | iex`
{% endif -%}
- Or run the `-setup.exe`. If SmartScreen says "Windows protected your PC", click
  **More info**, then **Run anyway**. It installs for your user only, no admin rights.
{% if updater %}
Installed apps update themselves from here once a release carries `latest.json`.
{% endif -%}
