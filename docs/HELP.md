# Reel — Help

## Playback

Use **File → Open** to choose a video. **Play** / **Pause** control playback. Drag the **timeline** or use the transport slider to seek.

## File

| Command        | Description                                      |
|----------------|--------------------------------------------------|
| Open           | Open a media file for preview.                   |
| Close          | Close the current media.                         |
| Revert         | Reload the file from disk (discards unsaved edits). |
| New Window     | Start another Reel process.                      |
| Save           | Save the current project (`.reel` JSON).         |
| Insert Video   | Insert a clip at the **playhead** on the main video track. If the playhead is **inside** a clip, that clip is split and the new clip is placed between the two parts. |
| Export         | Write the current media to another format.       |

## Edit

**Undo** / **Redo** apply to timeline and project edits as those features land.

## Window

**Always on top** keeps the player above other windows. **Fit**, **Fill**, and **Center** change how video fits the viewport.

## More

See the repository `README.md` for build instructions (`make setup`, `make run`).
