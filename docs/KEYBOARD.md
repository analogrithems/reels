# Reel — keyboard shortcuts (desktop)

Click the **video or timeline area** so the main view has focus for playback and editing shortcuts. **Open**, **Save** (when enabled), and **Close Window** (when enabled) work even before **media ready**. **F1** opens **Help → Overview** anytime the main view has focus. **Esc** exits **fullscreen** when the window is fullscreen; otherwise **Esc** closes the **Trim Clip…** sheet if it is open; otherwise **Esc** closes the **export preset** sheet if it is open; otherwise it cancels an **in-progress export** (same as **Cancel export** on the export progress modal). **Insert Video**, **Insert Audio** (when enabled), **Export**, **New Video Track**, and **New Audio Track** need the timeline/player ready (same as the **File** menu). If a shortcut does nothing, click the preview or timeline strip and try again.

## Help

| Action | Shortcut |
|--------|----------|
| **Help → Overview** | **F1** |

## File

| Action | Windows / Linux | macOS |
|--------|-----------------|-------|
| Open… | **Ctrl+O** | **⌘O** |
| Open Recent | *(menu only — no default shortcut)* | same |
| Save… | **Ctrl+S** (when Save is enabled) | **⌘S** |
| Close Window | **Ctrl+W** (when Close Window is enabled; not while exporting) | **⌘W** |
| Insert Video… | **Ctrl+I** (needs media ready) | **⌘I** |
| Insert Audio… | **Ctrl+Shift+I** (needs an audio track + media ready) | **⌘⇧I** |
| Export… | **Ctrl+E** (needs media ready; disabled while exporting or while the export preset sheet is open) | **⌘E** |
| Cancel in-progress export / dismiss export preset | **Esc** (while exporting, or while the export preset sheet is open). While encoding: **Cancel export** on the progress modal. | same |
| New Video Track | **Ctrl+Shift+N** (needs media ready) | **⌘⇧N** |
| New Audio Track | **Ctrl+Shift+A** (needs media ready) | **⌘⇧A** |

## View

| Action | Windows / Linux | macOS |
|--------|-----------------|-------|
| Toggle **Loop Playback** | **Ctrl+L** (works without media ready) | **⌘L** |
| **Show Status** (codec / path / save line) | *(menu only — no default shortcut)* | same |
| **Always Show Controls** (floating transport) | *(menu only — no default shortcut)* | same |
| **Zoom In** | **Ctrl+=** (works without media ready) | **⌘=** |
| **Zoom Out** | **Ctrl+-** (works without media ready) | **⌘-** |
| **Zoom to Fit** | **Ctrl+0** (works without media ready) | **⌘0** |
| Exit **fullscreen** | **Esc** when fullscreen | **Esc** |

## Playback & timeline

| Action | Shortcut |
|--------|----------|
| Play / pause | **Space** |
| Nudge playhead by ±1 second | **←** / **→** (clamped to the sequence) |
| Jump to sequence start / end | **Home** / **End** |

## Edit

Same actions as **Edit → Undo / Redo** when those menu items are enabled.

| Action | Windows / Linux | macOS |
|--------|-----------------|-------|
| Undo | **Ctrl+Z** | **⌘Z** |
| Redo | **Ctrl+Shift+Z** | **⌘⇧Z** |
| Split clip at playhead | **Ctrl+B** (when enabled) | **⌘B** |
| Rotate 90° right | **Ctrl+R** (when the playhead is on a primary-track clip) | **⌘R** |
| Rotate 90° left | **Ctrl+Shift+R** | **⌘⇧R** |
| **Trim Clip…** | **Edit → Trim Clip…** *(when trim is enabled — playhead on a primary-track clip)*; *no default shortcut* | same |
| **Set In Point** (range marker) | **I** *(no modifier; when media ready)* | **I** |
| **Set Out Point** (range marker) | **O** *(no modifier; when media ready)* | **O** |
| **Clear Range Markers** | **Alt+X** *(when at least one marker is set)* | **⌥X** |

Blade split only works when the playhead is **strictly inside** a clip on the primary track (not in a gap or on a cut). Same as **Edit → Split Clip at Playhead**. **Trim Clip…** opens a sheet for begin/end in source-file seconds; click outside the sheet card or **Cancel** to dismiss. Rotate/Flip apply to the clip under the playhead and persist in the project; **Flip Horizontal / Flip Vertical** are in the **Edit** menu without keyboard shortcuts. The **In/Out range markers** are ephemeral (not saved to the project), visible on the timeline slider (In = cyan, Out = magenta, with a tint spanning the range); setting In past the current Out (or Out before the current In) clears the conflicting marker.

## Multi-track editing

These match **Edit → Move Clip to Track Below / Above** when those menu items are enabled (see **Help → Features & roadmap** for rules).

| Action | Windows / Linux | macOS |
|--------|-----------------|-------|
| Move clip to track below | **Ctrl+Shift+↓** | **⌘⇧↓** |
| Move clip to track above | **Ctrl+Shift+↑** | **⌘⇧↑** |

## A/V sync offset

Nudge the audio clock's device-latency offset in 25 ms steps. Also exposed under **View → A/V Offset** with current value + reset. Works with or without media loaded (it's a per-device pref, persisted to `settings.json`).

| Action | Windows / Linux | macOS |
|--------|-----------------|-------|
| Shift audio earlier (−25 ms) | **Shift+←** | **⇧←** |
| Shift audio later (+25 ms) | **Shift+→** | **⇧→** |

On **macOS**, Slint's `KeyEvent` maps the **Command (⌘)** key to the `control` modifier field, so **Ctrl+Shift** in this table corresponds to **⌘⇧** on an Apple keyboard.

---

For the full product list (menus, export, effects), see **Help → Features & roadmap**.
