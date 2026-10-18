# Setup guide

This walks you from install to your first read-aloud in a couple of minutes.

## 1. Install

Download the build for your OS from the
[Releases](https://github.com/harshaldevvv/read-selected-text/releases) page:

- **Windows** — run the `.msi` or `-setup.exe`. Windows SmartScreen may warn
  about an unknown publisher (the app is unsigned); choose **More info → Run
  anyway**.
- **macOS** — open the `.dmg` and drag the app to Applications. On first launch,
  right-click the app → **Open** to bypass Gatekeeper (unsigned build).
- **Linux** — make the `.AppImage` executable (`chmod +x`) and run it, or
  install the `.deb` with `sudo apt install ./read-selected-text_*.deb`.

## 2. Choose a voice engine

Open the app (its window appears; it also lives in your system tray). Pick one:

### OpenAI (recommended default)

1. Create a key at <https://platform.openai.com/api-keys>.
2. Paste it into the **OpenAI → API key** field and click **Save key**.
3. Pick a model and voice. `gpt-4o-mini-tts` is a great default.

Roughly **$15 per 1,000,000 characters** — a full-length article costs a
fraction of a cent.

### ElevenLabs

1. Create a key at <https://elevenlabs.io/app/settings/api-keys>.
2. Paste it into **ElevenLabs → API key** and click **Save key**.
3. Paste a **Voice ID** from your [Voice Library](https://elevenlabs.io/app/voice-library)
   (the default is the "Rachel" voice).

### System voice (no key, offline)

Select **System voice**. Choose an installed voice and rate. Nothing leaves your
computer.

- **Windows** uses the SAPI voices from *Settings → Time & language → Speech*.
- **macOS** uses the voices from *System Settings → Accessibility → Spoken
  Content* (`say`).
- **Linux** requires `espeak-ng`: `sudo apt install espeak-ng`.

## 3. Test it

Click **▶ Test voice**. You should hear a sample sentence. Click **■ Stop** to
end playback at any time.

## 4. Use it anywhere

1. Select text in any app or website.
2. Press **Ctrl + Alt + R** (macOS: **Cmd + Alt + R**).
3. Listen. Press **Ctrl/Cmd + Alt + S** to stop.

You can change both hotkeys in **Settings → Hotkeys** (click a box and press
your combo).

## 5. Start at login

Tick **Start automatically when I log in** and click **Save settings**. The app
will launch quietly to the tray each time you sign in.

## Platform permissions

### macOS — Accessibility (required)

To copy your selection, the app sends a **Cmd+C** keystroke, which macOS gates
behind Accessibility permission.

- On first read, macOS prompts you. Approve it, **or**
- go to **System Settings → Privacy & Security → Accessibility** and enable
  **Read Selected Text**.

If reading returns "No text was selected" right after granting permission, quit
and reopen the app so the new permission takes effect.

### Windows

No special permission is needed. If a hotkey doesn't fire, another app may have
claimed the same combo — pick a different one in Settings.

### Linux

- Global shortcuts and synthetic keystrokes work best on **X11**. On **Wayland**,
  some compositors restrict global hotkeys and input synthesis; if the read
  hotkey doesn't trigger a copy, try an X11 session or use the tray menu's
  **Read selection** after copying manually.
- Install `espeak-ng` for the System voice engine.

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| "No text was selected" | Make sure text is highlighted; on macOS grant Accessibility and restart the app. |
| Hotkey does nothing | Another app may use the same combo — change it in Settings. |
| "No OpenAI/ElevenLabs API key set" | Add the key and click **Save key**. |
| No sound | Check system volume/output device; try **Test voice**. |
| Linux: nothing happens on System voice | `sudo apt install espeak-ng`. |

Your clipboard is restored after each read by default (toggle in **Playback**).
