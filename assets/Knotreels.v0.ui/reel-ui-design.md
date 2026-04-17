# Reel UI Design Specification

## Design Philosophy: iMovie Meets QuickTime

**Core Principle:** Combine iMovie's rich organizational features and media management with QuickTime's minimalist, distraction-free playback and trim controls.

### Design DNA

| Aspect | iMovie Influence | QuickTime Influence |
|--------|------------------|---------------------|
| **Layout** | Multi-panel with media library | Single-focus viewport |
| **Timeline** | Thumbnail filmstrip view | Simple scrub bar with trim markers |
| **Controls** | Rich transport with effects access | Minimal, auto-hiding controls |
| **Trim** | In-timeline handles | Yellow trim bar overlay |
| **Color** | Dark with accent colors | Neutral grays, clean borders |

---

## Color System

```
/* Reel Color Palette - macOS Native Feel */

/* Base Colors */
--background-primary: #1E1E1E;      /* Main app background */
--background-secondary: #2D2D2D;    /* Panel backgrounds */
--background-tertiary: #3D3D3D;     /* Elevated surfaces */
--background-hover: #4A4A4A;        /* Hover states */

/* Surface Colors */
--surface-timeline: #252525;        /* Timeline background */
--surface-preview: #000000;         /* Video preview area */
--surface-panel: #2A2A2A;           /* Side panels */

/* Border Colors */
--border-subtle: #3A3A3A;           /* Subtle dividers */
--border-default: #4A4A4A;          /* Default borders */
--border-focus: #0A84FF;            /* Focus rings */

/* Text Colors */
--text-primary: #FFFFFF;            /* Primary text */
--text-secondary: #A0A0A0;          /* Secondary/muted text */
--text-tertiary: #6E6E6E;           /* Disabled/hint text */

/* Accent Colors */
--accent-blue: #0A84FF;             /* Primary actions, selection */
--accent-green: #30D158;            /* Success, play state */
--accent-yellow: #FFD60A;           /* Trim handles (QuickTime-style) */
--accent-red: #FF453A;              /* Delete, stop, errors */
--accent-orange: #FF9F0A;           /* Warnings, export progress */

/* Timeline Specific */
--clip-video: #4A90D9;              /* Video clip color */
--clip-audio: #5AC8FA;              /* Audio clip color */
--playhead: #FFFFFF;                /* Playhead line */
--playhead-glow: rgba(255,255,255,0.3); /* Playhead glow effect */
```

---

## Layout Structure

```
┌─────────────────────────────────────────────────────────────────────────┐
│  Menu Bar (native macOS)                                                 │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │                                                                  │    │
│  │                                                                  │    │
│  │                     VIDEO PREVIEW                                │    │
│  │                  (QuickTime-style)                               │    │
│  │                                                                  │    │
│  │                                                                  │    │
│  │                  [Auto-hiding controls]                          │    │
│  │                                                                  │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │  Timecode: 00:01:23.456 / 00:05:30.000    [Speed ▼] [🔊 Vol]   │    │
│  ├─────────────────────────────────────────────────────────────────┤    │
│  │  ▶ ████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  │    │
│  │     ↑ Playhead                                                  │    │
│  ├─────────────────────────────────────────────────────────────────┤    │
│  │  TRACK 1 (Video)  │ [Clip 1 ████] [Clip 2 ████████] [Clip 3]   │    │
│  │  TRACK 2 (Audio)  │ [Audio ██████████████████████████████████]  │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │  Codec: H.264 / AAC  │  /path/to/project.reel  │  ✓ Saved      │    │
│  └─────────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## Component Specifications

### 1. Video Preview Area

**Style:** QuickTime-inspired with auto-hiding overlay controls

```slint
component VideoPreview inherits Rectangle {
    in property <image> frame;
    in property <bool> is-playing: false;
    in property <float> zoom-level: 1.0;
    in-out property <bool> controls-visible: false;
    
    callback play-pause();
    callback seek(float);
    
    background: #000000;
    
    // Video frame
    Image {
        source: frame;
        image-fit: contain;
        width: parent.width * zoom-level;
        height: parent.height * zoom-level;
        horizontal-alignment: center;
        vertical-alignment: center;
    }
    
    // Auto-hiding overlay controls (QuickTime-style)
    Rectangle {
        y: parent.height - 80px;
        width: parent.width;
        height: 80px;
        opacity: controls-visible ? 1.0 : 0.0;
        
        animate opacity { duration: 200ms; easing: ease-out; }
        
        // Gradient fade from transparent to semi-black
        background: @linear-gradient(180deg, transparent 0%, rgba(0,0,0,0.7) 100%);
        
        HorizontalLayout {
            alignment: center;
            spacing: 20px;
            padding: 20px;
            
            // Large centered play/pause button
            PlayPauseButton {
                is-playing: root.is-playing;
                clicked => { root.play-pause(); }
            }
        }
    }
    
    // Mouse interaction for showing/hiding controls
    TouchArea {
        moved => { 
            controls-visible = true;
            // Reset hide timer in Rust
        }
    }
}
```

### 2. Transport Bar (iMovie + QuickTime Hybrid)

```slint
component TransportBar inherits Rectangle {
    in property <duration> current-time;
    in property <duration> total-duration;
    in property <float> playhead-position;  // 0.0 to 1.0
    in property <float> volume: 1.0;
    in property <bool> is-playing: false;
    in property <bool> is-looping: false;
    in property <string> playback-speed: "1×";
    
    callback play-pause();
    callback seek(float);
    callback volume-changed(float);
    callback toggle-loop();
    callback speed-changed(string);
    
    height: 48px;
    background: #252525;
    border-radius: 0;
    
    VerticalLayout {
        spacing: 0;
        
        // Timecode and controls row
        HorizontalLayout {
            height: 24px;
            padding-left: 16px;
            padding-right: 16px;
            alignment: space-between;
            
            // Left: Timecode display
            HorizontalLayout {
                spacing: 8px;
                alignment: start;
                
                Text {
                    text: format-timecode(current-time);
                    color: #FFFFFF;
                    font-size: 12px;
                    font-family: "SF Mono", "Menlo", monospace;
                }
                Text {
                    text: "/";
                    color: #6E6E6E;
                    font-size: 12px;
                }
                Text {
                    text: format-timecode(total-duration);
                    color: #A0A0A0;
                    font-size: 12px;
                    font-family: "SF Mono", "Menlo", monospace;
                }
            }
            
            // Right: Speed selector and volume
            HorizontalLayout {
                spacing: 16px;
                alignment: end;
                
                // Playback speed dropdown
                SpeedSelector {
                    current-speed: playback-speed;
                    changed(speed) => { root.speed-changed(speed); }
                }
                
                // Loop toggle
                IconButton {
                    icon: @image-url("icons/loop.svg");
                    active: is-looping;
                    clicked => { root.toggle-loop(); }
                    tooltip: "Loop Playback (⌘L)";
                }
                
                // Volume control
                VolumeControl {
                    value: volume;
                    changed(v) => { root.volume-changed(v); }
                }
            }
        }
        
        // Scrub bar / Timeline strip
        Rectangle {
            height: 24px;
            background: #1E1E1E;
            
            HorizontalLayout {
                padding-left: 8px;
                padding-right: 8px;
                spacing: 8px;
                
                // Play/Pause button (compact)
                IconButton {
                    width: 24px;
                    icon: is-playing ? @image-url("icons/pause.svg") : @image-url("icons/play.svg");
                    clicked => { root.play-pause(); }
                }
                
                // Skip backward
                IconButton {
                    width: 20px;
                    icon: @image-url("icons/skip-back.svg");
                    clicked => { /* Jump to start */ }
                }
                
                // Scrub slider
                ScrubSlider {
                    value: playhead-position;
                    changed(pos) => { root.seek(pos); }
                }
                
                // Skip forward
                IconButton {
                    width: 20px;
                    icon: @image-url("icons/skip-forward.svg");
                    clicked => { /* Jump to end */ }
                }
            }
        }
    }
}
```

### 3. Timeline View (iMovie-style Filmstrip)

```slint
component Timeline inherits Rectangle {
    in property <[ClipModel]> video-clips;
    in property <[ClipModel]> audio-clips;
    in property <float> playhead-position;
    in property <duration> total-duration;
    in property <float> zoom-level: 1.0;
    
    callback clip-selected(int);
    callback clip-moved(int, float);
    callback playhead-dragged(float);
    callback split-at-playhead();
    
    background: #1E1E1E;
    min-height: 120px;
    
    VerticalLayout {
        spacing: 2px;
        padding: 8px;
        
        // Track header + clips
        for track-index in 2: Rectangle {
            height: 48px;
            background: #252525;
            border-radius: 4px;
            
            HorizontalLayout {
                spacing: 0;
                
                // Track label (fixed width)
                Rectangle {
                    width: 100px;
                    background: #2D2D2D;
                    border-radius: 4px 0 0 4px;
                    
                    HorizontalLayout {
                        padding: 8px;
                        spacing: 8px;
                        
                        // Track icon
                        Image {
                            source: track-index == 0 
                                ? @image-url("icons/video-track.svg")
                                : @image-url("icons/audio-track.svg");
                            width: 16px;
                            colorize: track-index == 0 ? #4A90D9 : #5AC8FA;
                        }
                        
                        Text {
                            text: track-index == 0 ? "Video" : "Audio";
                            color: #A0A0A0;
                            font-size: 11px;
                            vertical-alignment: center;
                        }
                    }
                }
                
                // Clips area (scrollable)
                Rectangle {
                    background: transparent;
                    clip: true;
                    
                    // Clip thumbnails (filmstrip style)
                    HorizontalLayout {
                        spacing: 2px;
                        
                        for clip in (track-index == 0 ? video-clips : audio-clips): ClipThumbnail {
                            clip-data: clip;
                            track-type: track-index == 0 ? "video" : "audio";
                            selected => { root.clip-selected(clip.id); }
                        }
                    }
                    
                    // Playhead overlay
                    Rectangle {
                        x: parent.width * playhead-position;
                        width: 2px;
                        height: parent.height;
                        background: #FFFFFF;
                        
                        // Glow effect
                        drop-shadow-blur: 4px;
                        drop-shadow-color: rgba(255, 255, 255, 0.5);
                    }
                }
            }
        }
    }
}
```

### 4. Clip Thumbnail (iMovie Filmstrip Style)

```slint
component ClipThumbnail inherits Rectangle {
    in property <ClipModel> clip-data;
    in property <string> track-type: "video";
    in property <bool> selected: false;
    
    callback selected();
    callback trim-start(float);
    callback trim-end(float);
    
    height: 40px;
    min-width: clip-data.duration-px;
    border-radius: 4px;
    background: track-type == "video" ? #4A90D9 : #5AC8FA;
    border-width: selected ? 2px : 0;
    border-color: #FFD60A;
    
    clip: true;
    
    HorizontalLayout {
        spacing: 0;
        
        // Filmstrip thumbnails (for video)
        if track-type == "video": HorizontalLayout {
            for thumb in clip-data.thumbnails: Image {
                source: thumb;
                width: 40px;
                image-fit: cover;
            }
        }
        
        // Waveform (for audio)
        if track-type == "audio": Rectangle {
            background: transparent;
            
            // Audio waveform visualization
            Path {
                // Waveform path data generated from audio analysis
                stroke: #FFFFFF;
                stroke-width: 1px;
            }
        }
    }
    
    // Clip name overlay
    Rectangle {
        y: parent.height - 16px;
        width: parent.width;
        height: 16px;
        background: @linear-gradient(180deg, transparent 0%, rgba(0,0,0,0.7) 100%);
        
        Text {
            text: clip-data.name;
            color: #FFFFFF;
            font-size: 9px;
            padding-left: 4px;
            vertical-alignment: center;
            overflow: elide;
        }
    }
    
    // Selection/hover state
    TouchArea {
        clicked => { root.selected(); }
    }
    
    // Trim handles (appear on hover/selection)
    if selected: Rectangle {
        x: 0;
        width: 6px;
        height: parent.height;
        background: #FFD60A;
        border-radius: 3px 0 0 3px;
        
        TouchArea {
            mouse-cursor: ew-resize;
            // Handle trim start drag
        }
    }
    
    if selected: Rectangle {
        x: parent.width - 6px;
        width: 6px;
        height: parent.height;
        background: #FFD60A;
        border-radius: 0 3px 3px 0;
        
        TouchArea {
            mouse-cursor: ew-resize;
            // Handle trim end drag
        }
    }
}
```

### 5. QuickTime-Style Trim Mode

When user triggers trim (Cmd+T or Edit > Trim), the interface transforms:

```slint
component TrimMode inherits Rectangle {
    in property <image> frame;
    in property <duration> duration;
    in-out property <duration> trim-start: 0;
    in-out property <duration> trim-end: duration;
    
    callback apply-trim();
    callback cancel-trim();
    
    background: #000000;
    
    VerticalLayout {
        // Video preview
        Image {
            source: frame;
            image-fit: contain;
            preferred-height: parent.height - 100px;
        }
        
        // Trim bar (QuickTime yellow handles)
        Rectangle {
            height: 100px;
            background: #1E1E1E;
            
            VerticalLayout {
                spacing: 8px;
                padding: 16px;
                
                // Filmstrip with trim handles
                Rectangle {
                    height: 50px;
                    background: #2D2D2D;
                    border-radius: 6px;
                    
                    // Thumbnail strip
                    HorizontalLayout {
                        clip: true;
                        
                        for thumb in clip-thumbnails: Image {
                            source: thumb;
                            width: 50px;
                            image-fit: cover;
                        }
                    }
                    
                    // Dimmed areas outside trim range
                    Rectangle {
                        x: 0;
                        width: trim-start / duration * parent.width;
                        background: rgba(0, 0, 0, 0.6);
                    }
                    
                    Rectangle {
                        x: trim-end / duration * parent.width;
                        width: (duration - trim-end) / duration * parent.width;
                        background: rgba(0, 0, 0, 0.6);
                    }
                    
                    // Yellow trim frame (QuickTime signature)
                    Rectangle {
                        x: trim-start / duration * parent.width;
                        width: (trim-end - trim-start) / duration * parent.width;
                        border-width: 3px;
                        border-color: #FFD60A;
                        border-radius: 4px;
                        background: transparent;
                        
                        // Left handle
                        Rectangle {
                            x: -8px;
                            width: 16px;
                            height: parent.height;
                            background: #FFD60A;
                            border-radius: 4px 0 0 4px;
                            
                            // Grip lines
                            VerticalLayout {
                                alignment: center;
                                spacing: 3px;
                                
                                for i in 3: Rectangle {
                                    width: 4px;
                                    height: 1px;
                                    background: #000000;
                                }
                            }
                            
                            TouchArea {
                                mouse-cursor: ew-resize;
                                moved(event) => {
                                    // Update trim-start
                                }
                            }
                        }
                        
                        // Right handle
                        Rectangle {
                            x: parent.width - 8px;
                            width: 16px;
                            height: parent.height;
                            background: #FFD60A;
                            border-radius: 0 4px 4px 0;
                            
                            // Grip lines
                            VerticalLayout {
                                alignment: center;
                                spacing: 3px;
                                
                                for i in 3: Rectangle {
                                    width: 4px;
                                    height: 1px;
                                    background: #000000;
                                }
                            }
                            
                            TouchArea {
                                mouse-cursor: ew-resize;
                                moved(event) => {
                                    // Update trim-end
                                }
                            }
                        }
                    }
                }
                
                // Action buttons
                HorizontalLayout {
                    alignment: center;
                    spacing: 16px;
                    
                    Button {
                        text: "Cancel";
                        style: "secondary";
                        clicked => { root.cancel-trim(); }
                    }
                    
                    Button {
                        text: "Trim";
                        style: "primary";
                        clicked => { root.apply-trim(); }
                    }
                }
            }
        }
    }
}
```

### 6. Status Footer

```slint
component StatusFooter inherits Rectangle {
    in property <string> video-codec: "H.264";
    in property <string> audio-codec: "AAC";
    in property <string> clip-path;
    in property <string> project-path;
    in property <bool> is-saved: true;
    
    height: 24px;
    background: #1E1E1E;
    border-width: 1px 0 0 0;
    border-color: #3A3A3A;
    
    HorizontalLayout {
        padding-left: 12px;
        padding-right: 12px;
        spacing: 0;
        
        // Codec info
        HorizontalLayout {
            width: 150px;
            spacing: 8px;
            
            Text {
                text: video-codec;
                color: #A0A0A0;
                font-size: 11px;
                vertical-alignment: center;
            }
            
            Rectangle {
                width: 1px;
                height: 12px;
                background: #3A3A3A;
            }
            
            Text {
                text: audio-codec;
                color: #A0A0A0;
                font-size: 11px;
                vertical-alignment: center;
            }
        }
        
        Rectangle {
            width: 1px;
            height: 16px;
            background: #3A3A3A;
        }
        
        // File paths
        Text {
            text: project-path != "" ? project-path : "Not saved to disk";
            color: project-path != "" ? #A0A0A0 : #6E6E6E;
            font-size: 11px;
            horizontal-stretch: 1;
            overflow: elide;
            horizontal-alignment: center;
            vertical-alignment: center;
        }
        
        Rectangle {
            width: 1px;
            height: 16px;
            background: #3A3A3A;
        }
        
        // Save status
        HorizontalLayout {
            width: 130px;
            spacing: 6px;
            alignment: end;
            
            Image {
                source: is-saved 
                    ? @image-url("icons/check-circle.svg")
                    : @image-url("icons/circle.svg");
                width: 12px;
                colorize: is-saved ? #30D158 : #6E6E6E;
            }
            
            Text {
                text: is-saved ? "All changes saved" : "Unsaved changes";
                color: is-saved ? #30D158 : #A0A0A0;
                font-size: 11px;
                vertical-alignment: center;
            }
        }
    }
}
```

### 7. Export Progress Bar

```slint
component ExportProgress inherits Rectangle {
    in property <float> progress: 0.0;  // 0.0 to 1.0
    in property <string> status: "Exporting...";
    in property <bool> visible: false;
    
    callback cancel-export();
    
    height: visible ? 4px : 0;
    background: #2D2D2D;
    
    animate height { duration: 200ms; easing: ease-out; }
    
    Rectangle {
        width: parent.width * progress;
        height: parent.height;
        background: @linear-gradient(90deg, #0A84FF 0%, #5AC8FA 100%);
        
        animate width { duration: 100ms; }
    }
}
```

---

## Main Application Window

```slint
import { Button, VerticalBox, HorizontalBox } from "std-widgets.slint";

export component ReelApp inherits Window {
    title: "Reel";
    icon: @image-url("icons/reel-app.svg");
    min-width: 800px;
    min-height: 600px;
    background: #1E1E1E;
    
    // State properties
    in-out property <image> current-frame;
    in-out property <bool> is-playing: false;
    in-out property <duration> current-time: 0;
    in-out property <duration> total-duration: 0;
    in-out property <float> volume: 1.0;
    in-out property <bool> is-looping: false;
    in-out property <float> zoom-level: 1.0;
    in-out property <bool> trim-mode: false;
    in-out property <float> export-progress: 0.0;
    in-out property <bool> is-exporting: false;
    
    // Clip data
    in-out property <[ClipModel]> video-clips: [];
    in-out property <[ClipModel]> audio-clips: [];
    
    // Status
    in-out property <string> video-codec: "";
    in-out property <string> audio-codec: "";
    in-out property <string> project-path: "";
    in-out property <bool> is-saved: true;
    
    // Callbacks
    callback play-pause();
    callback seek(float);
    callback open-file();
    callback save-project();
    callback export-video();
    callback split-clip();
    callback toggle-loop();
    
    VerticalLayout {
        spacing: 0;
        
        // Export progress (appears when exporting)
        ExportProgress {
            progress: export-progress;
            visible: is-exporting;
        }
        
        // Main content area
        if !trim-mode: VerticalLayout {
            spacing: 0;
            
            // Video Preview
            VideoPreview {
                frame: current-frame;
                is-playing: root.is-playing;
                zoom-level: root.zoom-level;
                vertical-stretch: 1;
                
                play-pause => { root.play-pause(); }
                seek(pos) => { root.seek(pos); }
            }
            
            // Transport Bar
            TransportBar {
                current-time: root.current-time;
                total-duration: root.total-duration;
                playhead-position: total-duration > 0 
                    ? current-time / total-duration 
                    : 0.0;
                volume: root.volume;
                is-playing: root.is-playing;
                is-looping: root.is-looping;
                
                play-pause => { root.play-pause(); }
                seek(pos) => { root.seek(pos); }
                volume-changed(v) => { root.volume = v; }
                toggle-loop => { root.toggle-loop(); }
            }
            
            // Timeline
            Timeline {
                video-clips: root.video-clips;
                audio-clips: root.audio-clips;
                playhead-position: total-duration > 0 
                    ? current-time / total-duration 
                    : 0.0;
                total-duration: root.total-duration;
                
                min-height: 120px;
                max-height: 200px;
                
                playhead-dragged(pos) => { root.seek(pos); }
                split-at-playhead => { root.split-clip(); }
            }
            
            // Status Footer
            StatusFooter {
                video-codec: root.video-codec;
                audio-codec: root.audio-codec;
                project-path: root.project-path;
                is-saved: root.is-saved;
            }
        }
        
        // Trim mode view
        if trim-mode: TrimMode {
            frame: current-frame;
            duration: total-duration;
            
            apply-trim => { 
                root.trim-mode = false;
                // Apply trim logic
            }
            cancel-trim => { 
                root.trim-mode = false; 
            }
        }
    }
    
    // Keyboard shortcuts
    // Note: Implement FocusScope for keyboard handling
}

// Data structures
struct ClipModel {
    id: int,
    name: string,
    path: string,
    in-point: duration,
    out-point: duration,
    duration-px: length,  // Width in timeline pixels
    thumbnails: [image],
    rotation: int,       // 0, 90, 180, 270
    flip-h: bool,
    flip-v: bool,
}
```

---

## Icon Set (Required)

Create these SVG icons for consistent UI:

| Icon | Usage | Style |
|------|-------|-------|
| `play.svg` | Play button | Filled triangle |
| `pause.svg` | Pause button | Two vertical bars |
| `skip-back.svg` | Jump to start | Double left arrows |
| `skip-forward.svg` | Jump to end | Double right arrows |
| `loop.svg` | Loop toggle | Circular arrow |
| `volume.svg` | Volume control | Speaker with waves |
| `volume-mute.svg` | Muted | Speaker with X |
| `video-track.svg` | Video track | Film strip frame |
| `audio-track.svg` | Audio track | Waveform |
| `check-circle.svg` | Saved status | Checkmark in circle |
| `circle.svg` | Unsaved status | Empty circle |
| `split.svg` | Split clip | Blade/scissors |
| `rotate-cw.svg` | Rotate right | Circular arrow CW |
| `rotate-ccw.svg` | Rotate left | Circular arrow CCW |
| `flip-h.svg` | Flip horizontal | Mirror arrows |
| `flip-v.svg` | Flip vertical | Up/down arrows |

---

## Animation Guidelines

```slint
/* Standard timing curves */
--ease-default: cubic-bezier(0.25, 0.1, 0.25, 1.0);   /* General transitions */
--ease-out: cubic-bezier(0.0, 0.0, 0.2, 1.0);         /* Elements appearing */
--ease-in: cubic-bezier(0.4, 0.0, 1.0, 1.0);          /* Elements disappearing */
--ease-in-out: cubic-bezier(0.4, 0.0, 0.2, 1.0);      /* Moving elements */

/* Durations */
--duration-fast: 100ms;    /* Micro-interactions (button press) */
--duration-normal: 200ms;  /* Standard transitions */
--duration-slow: 300ms;    /* Complex animations */

/* Usage in Slint */
animate opacity { 
    duration: 200ms; 
    easing: ease-out; 
}

animate background { 
    duration: 100ms; 
    easing: ease-in-out; 
}
```

---

## Responsive Behavior

### Window Size Breakpoints

| Width | Layout Changes |
|-------|----------------|
| < 800px | Hide codec labels in footer, compact timeline |
| 800-1200px | Standard layout |
| > 1200px | Expanded timeline with larger thumbnails |

### Timeline Scaling

- Default: 10 pixels per second
- Zoom levels: 5, 10, 20, 40, 80 px/sec
- Filmstrip thumbnail width: 40-80px based on zoom

---

## Accessibility

1. **Keyboard Navigation**
   - Tab through interactive elements
   - Arrow keys for timeline navigation
   - Space for play/pause
   - Full shortcut support per KEYBOARD.md

2. **Screen Reader Support**
   - All interactive elements have accessible names
   - Announce playback state changes
   - Describe clip positions and durations

3. **Visual Accessibility**
   - Minimum 4.5:1 contrast ratio for text
   - Clear focus indicators (2px blue outline)
   - No reliance on color alone for information

---

## Platform-Specific Notes

### macOS
- Use native menu bar via Slint's platform integration
- Support trackpad gestures (pinch to zoom, two-finger scrub)
- Respect system dark/light mode (future enhancement)
- Native file dialogs via rfd or similar

### Future: Windows/Linux
- In-window menu bar with same structure
- Mouse-only interactions initially
- GTK/Qt dialogs depending on backend
