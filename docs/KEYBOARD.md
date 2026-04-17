# Reel — keyboard shortcuts (desktop)

Click the **video or timeline area** so the main view has focus for playback and editing shortcuts. **Open**, **Save** (when enabled), and **Close** (when enabled) work even before **media ready**. **F1** opens **Help → Overview** anytime the main view has focus. **Esc** closes the **export preset** sheet if it is open; otherwise it cancels an **in-progress export** (same as **File → Cancel Export**). **Insert Video**, **Insert Audio** (when enabled), **Export**, **New Video Track**, and **New Audio Track** need the timeline/player ready (same as the **File** menu). If a shortcut does nothing, click the preview or timeline strip and try again.

## Help

| Action | Shortcut |
|--------|----------|
| **Help → Overview** | **F1** |

## File

| Action | Windows / Linux | macOS |
|--------|-----------------|-------|
| Open… | **Ctrl+O** | **⌘O** |
| Save… | **Ctrl+S** (when Save is enabled) | **⌘S** |
| Close | **Ctrl+W** (when Close is enabled) | **⌘W** |
| Insert Video… | **Ctrl+I** (needs media ready) | **⌘I** |
| Insert Audio… | **Ctrl+Shift+I** (needs an audio track + media ready) | **⌘⇧I** |
| Export… | **Ctrl+E** (needs media ready; disabled while exporting or while the export preset sheet is open) | **⌘E** |
| Cancel Export / dismiss export preset | **Esc** (while exporting, or while the export preset sheet is open) | **Esc** |
| New Video Track | **Ctrl+Shift+N** (needs media ready) | **⌘⇧N** |
| New Audio Track | **Ctrl+Shift+A** (needs media ready) | **⌘⇧A** |

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

Blade split only works when the playhead is **strictly inside** a clip on the primary track (not in a gap or on a cut). Same as **Edit → Split Clip at Playhead**.

## Multi-track editing

These match **Edit → Move Clip to Track Below / Above** when those menu items are enabled (see **Help → Features & roadmap** for rules).

| Action | Windows / Linux | macOS |
|--------|-----------------|-------|
| Move clip to track below | **Ctrl+Shift+↓** | **⌘⇧↓** |
| Move clip to track above | **Ctrl+Shift+↑** | **⌘⇧↑** |

On **macOS**, Slint’s `KeyEvent` maps the **Command (⌘)** key to the `control` modifier field, so **Ctrl+Shift** in this table corresponds to **⌘⇧** on an Apple keyboard.

---

For the full product list (menus, export, effects), see **Help → Features & roadmap**.
