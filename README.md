<div align="center">
<img
    src="https://capsule-render.vercel.app/api?type=waving&color=0:020617,25:082f49,55:075985,80:0891b2,100:67e8f9&height=220&section=header&text=WuWa-APR&fontSize=60&fontColor=b3deff&fontAlignY=50&animation=fadeIn"
    width="100%"
  />
  
#### It automatically restarts the game after a patch, because the game itself doesn't.
[![Release](https://img.shields.io/github/v/release/KurumiZph/WuWa-AutoPatchReboot?style=flat-square&color=00BF63)](../../releases)
[![Downloads](https://img.shields.io/github/downloads/KurumiZph/WuWa-AutoPatchReboot/total?style=flat-square&color=4C9AFF)](../../releases)
[![Platform](https://img.shields.io/badge/Windows-10%20%7C%2011-0078D6?style=flat-square)](#get-started)
[![License](https://img.shields.io/github/license/KurumiZph/WuWa-AutoPatchReboot?style=flat-square)](LICENSE)

[Download](../../releases) · [Report a bug](../../issues) · [Build it yourself](DEVELOPMENT.md)

<img src="images/ui.png" alt="WuWa APR" width="720">

</div>

## What it does

Use WuWa APR as your game shortcut. It launches the game, reads the screen with Windows OCR, and handles the rest:

- **Patch screen?** Closes the game and starts it again. A small notification box shows while it restarts.
- **Login screen?** Stops there, so you can log in yourself (or auto-login, up to you).
- **Already in game?** Detects your UID on screen and closes itself.

## Get started

You will need a Windows OCR language for your game language (English is usually there already).

1. Download `wuwa-apr.exe` from [Releases](../../releases).
2. Run it (it asks for admin) and pick your game install if it asks. It launches the game and does the rest.
3. From now on, start the game through `wuwa-apr.exe` (pin it or make a desktop shortcut) instead of the game's own
   shortcut.

It sits in the tray with a menu: **Status**, **Show UI** and **Quit**.
Quitting the app does not close the game.

 >#### Supported Languages:
| Language | Login | Patch | UID |
| - | - | - | - |
| English | ✔ | ✔ | ✔ |
| French | ✔ | ❌ | ✔ |
| German | ✔ | ❌ | ✔ |
| Spanish | ✔ | ❌ | ✔ |
| Portuguese | ✔ | ❌ | ✔ |
| Japanese | ✔ | ❌ | ✔ |
| Korean | ✔ | ❌ | ✔ |
| Chinese - S | ✔ | ❌ | ✔ |
| Chinese - T | ✔ | ❌ | ✔ |
| Thai | ✔ | ❌ | ✔ |
## The window

- **Live Logs:** what APR is doing right now. Select text, copy it, export it, or filter by colour.
- **Settings:** presets, scan and startup values, Auto Login, the notification box, and Debugging options. Hover a
  setting for an explanation. Settings can be imported and exported.
- **Misc:** tools to test detection without waiting for a real patch (self-test, test an image), reload
  languages, and open your data folder.

The dot in the footer shows the state: **green** is running normally, **blue** is finished (APR is waiting for you
to close the window), **red** is an error.

## Good to know

- **Presets:**
>- Eco - slower reaction to screen updates to conserve cpu load.
>- Balanced - balance between recognition speed and cpu usage.
>- Turbo - Snappy reactions to frame updates for minimal delays - higher cpu usage.
- **Languages:** English, French, German, Spanish, Portuguese, Japanese, Korean, Chinese (Simplified / Traditional)
  and Thai.
>⚠ Login and UID detection works in all languages but **patch detection only works in English.** <br> (see the [Roadmap](#roadmap)).
- **Safety:** a restart-loop guard stops APR if the game keeps restarting.
- **Auto Login:** off by default. It brings the game to the front and sends one click or Space press.
- **Notification box:** its size, opacity and position are in Settings, and it previews live while you change them.
  It can't draw over exclusive fullscreen.

<details>
<summary><b>Other languages</b></summary>

<br>

Japanese, Korean, Chinese and Thai need their Windows OCR language installed (Settings shows a green / red dot):

```powershell
Add-WindowsCapability -Online -Name "Language.OCR~~~ja-JP~0.0.1.0"
```

Patch texts are only included for English so far. Texts live in `%APPDATA%\WuWa-APR\languages.json`; add yours there
and press **Misc > Reload languages**.

</details>

<details>
<summary><b>Troubleshooting</b></summary>

<br>

- **Not detecting anything?** Turn on *Save screenshots* (Settings > Debugging), open **Misc > Latest screenshot**,
  then try **Test an image** on it.
- **Wrong game install?** Delete `%APPDATA%\WuWa-APR\config.json` and start again.
- **Nothing happens?** A minimized game can't be captured.
- **Didn't close itself in game?** UID detection can miss the small text in the bottom-right corner. It is on the
  [Roadmap](#roadmap). Turn on *Save screenshots* and send an [issue](../../issues) with a log.
- **Patch not detected in your language?** Only English patch texts are built in. See *Other languages* above.
- **No notification box?** It can't draw over exclusive fullscreen. Windowed / borderless works.

</details>

<details>
<summary><b>Where are my files?</b></summary>

<br>

`%APPDATA%\WuWa-APR\` holds `config.json`, `settings.json`, `languages.json`, `logs\`, `screenshots\` and `exports\`.

</details>

## Roadmap

Things that are planned but not done yet. Help with the language texts is very welcome.

- [ ] **Patch notice texts for more languages.** Only English is included so far.
  <br>**Missing:** French, German, Spanish, Portuguese, Japanese, Korean, Simplified Chinese, Traditional Chinese and Thai. Login and UID texts are already in all ten languages.
> (because the notice appears unknowingly on a patch and can't be triggered on demand, I don't have text for other languages. Any help is greatly appreciated.)
  
- [ ] **Automatic language detection**, by recognising the words of the photosensitivity warning screen.
- [ ] **More reliable UID detection** (a sharper look at the bottom-right corner of the screen).
- [ ] **Region-based scanning**: read only the part of the window that matters for each check, for speed and cleaner logs.
- [ ] **UI Enhancements**
- [ ] **Auto login support while in background**
- [ ] **Update check | Auto Updates** in the app.
- [ ] **(Steam) Launching through the Steam UI** instead of using APR as the game shortcut.

Got a patch screenshot or text in your language? Open an [issue](../../issues) or add it to your own `languages.json`
and share it.

## Disclaimer

Unofficial fan project, not affiliated with Kuro Games or Wuthering Waves. Use at your own risk.
Parts of it were written with AI assistance; it's maintained and tested by the repo owner.

## License

[MIT](LICENSE)