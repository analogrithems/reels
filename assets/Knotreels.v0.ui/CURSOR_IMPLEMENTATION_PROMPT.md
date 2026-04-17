# Cursor Implementation Prompt: Reel Video Editor UI in Slint

You are implementing the UI for **Reel**, an open-source video editing application built with Rust and [Slint](https://slint.dev). The design should feel like a hybrid of **iMovie** (organizational features, timeline filmstrip) and **QuickTime Player** (minimalist controls, yellow trim handles).

## Project Context

- **Repository**: https://github.com/analogrithems/reels
- **Framework**: Slint UI (declarative, reactive, Rust-native)
- **Target Platforms**: macOS (primary), with future Windows/Linux support
- **Design Philosophy**: Clean, distraction-free editing with pro-level features accessible via keyboard shortcuts

---

## Visual Design Specification

### Color Palette (Dark Theme, macOS-Native Feel)

```slint
// Define these as global tokens
global Theme {
    // Backgrounds
    out property <color> background-primary: #1E1E1E;
    out property <color> background-secondary: #2D2D2D;
    out property <color> background-tertiary: #3D3D3D;
    out property <color> background-hover: #4A4A4A;
    out property <color> surface-timeline: #252525;
    out property <color> surface-preview: #000000;
    
    // Text
    out property <color> text-primary: #FFFFFF;
    out property <color> text-secondary: #A0A0A0;
    out property <color> text-tertiary: #6E6E6E;
    
    // Borders
    out property <color> border-subtle: #3A3A3A;
    out property <color> border-default: #4A4A4A;
    out property <color> border-focus: #0A84FF;
    
    // Accents
    out property <color> accent-blue: #0A84FF;      // Primary actions, selection
    out property <color> accent-green: #30D158;     // Success, play state
    out property <color> accent-yellow: #FFD60A;    // QuickTime-style trim handles
    out property <color> accent-red: #FF453A;       // Delete, errors
    out property <color> accent-orange: #FF9F0A;    // Subtitles, warnings
    
    // Track colors
    out property <color> clip-video: #4A90D9;
    out property <color> clip-audio: #5AC8FA;
    out property <color> clip-subtitle: #FF9F0A;
}
```

### Typography

- **Primary font**: System font (SF Pro on macOS, Segoe on Windows)
- **Monospace** (timecode): SF Mono, JetBrains Mono, or Menlo
- **Sizes**: 9px (clip labels), 11px (footer/secondary), 12px (body), 14px (headers)

---

## Application Layout Structure

The video preview area extends to the bottom of the window when all tracks are hidden. The floating controls overlay the video and auto-hide after 5 seconds of inactivity.

```
┌─────────────────────────────────────────────────────────────────────────┐
│  [Menu Bar: File, Edit, View, Effects, Window, Help]                    │
├─────────────────────────────────────────────────────────────────────────┤
│  [Export Progress Bar - only visible during export]                      │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │                                                                  │    │
│  │                     VIDEO PREVIEW                                │    │
│  │                  (fills available space)                         │    │
│  │                                                                  │    │
│  │    ┌─────────────────────────────────────────────────────┐      │    │
│  │    │ FLOATING CONTROLS (60% width, draggable, auto-hide) │      │    │
│  │    │ [≡] [⏮] [◀][▶/⏸][▶] [⏭] │ [1×▼][↺][🔊▬][⛶] [»]   │      │    │
│  │    │ 00:01:23 ════════════●════════════════════ 05:30:00 │      │    │
│  │    └─────────────────────────────────────────────────────┘      │    │
│  │                                                                  │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │  TIMELINE (px-4 py-3 padding)                            [+] Video│   │
│  │  ┌──────────────┬────────────────────────────────────────────┐  │    │
│  │  │ 🎬 Video1 [x]│ [Clip1 ████] [Clip2 ████████████] [Clip3]  │  │    │
│  │  └──────────────┴────────────────────────────────────────────┘  │    │
│  │                                                          [+] Audio│   │
│  │  ┌──────────────┬────────────────────────────────────────────┐  │    │
│  │  │ 🎵 Audio1 [x]│ [~~~waveform~~~~~~~~~~~~~~~~~~~~~~~~~~~]   │  │    │
│  │  └──────────────┴────────────────────────────────────────────┘  │    │
│  │              ▼ (playhead - white line with glow)                 │    │
│  └─────────────────────────────────────────────────────────────────┘    │
│  (Subtitles hidden by default - enable via View menu)                   │
│                                                                          │
│  ┌─────────────────────────────────────────────────────────────────┐    │
│  │  H.264 | AAC    │    ~/Projects/MyVideo.reel    │   ✓ Saved     │    │
│  └─────────────────────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────────────────────┘
```

**Key Layout Features:**
- Video preview fills to bottom when Timeline and/or Status Bar are hidden
- Floating controls are 60% width, centered at bottom 10%, draggable anywhere
- Track labels show delete button [x] on hover
- Timeline has `px-4 py-3` padding for readability
- Subtitles hidden by default (toggle via View > Subtitle Tracks)

---

## Component Implementation Guide

### 1. Main Application Window (`ui/app.slint`)

```slint
import { VerticalBox, HorizontalBox } from "std-widgets.slint";
import { Theme } from "theme.slint";
import { VideoPreview } from "components/video-preview.slint";
import { TransportBar } from "components/transport-bar.slint";
import { Timeline } from "components/timeline.slint";
import { StatusFooter } from "components/status-footer.slint";
import { ExportProgress } from "components/export-progress.slint";
import { TrimMode } from "components/trim-mode.slint";

export struct ClipModel {
    id: int,
    name: string,
    duration-ms: int,
    color: color,
    thumbnails: [image],
}

export struct TrackModel {
    id: int,
    name: string,
    track-type: string,  // "video", "audio", "subtitle"
    clips: [ClipModel],
}

export component ReelApp inherits Window {
    title: "Reel";
    min-width: 800px;
    min-height: 600px;
    background: Theme.background-primary;
    
    // State
    in-out property <image> current-frame;
    in-out property <bool> is-playing: false;
    in-out property <int> current-time-ms: 0;
    in-out property <int> total-duration-ms: 0;
    in-out property <float> volume: 1.0;
    in-out property <bool> is-muted: false;
    in-out property <bool> is-looping: false;
    in-out property <string> playback-speed: "1×";
    in-out property <bool> trim-mode: false;
    in-out property <bool> is-exporting: false;
    in-out property <float> export-progress: 0.0;
    
    // Track data - supports multiple tracks per type
    in-out property <[TrackModel]> video-tracks: [];
    in-out property <[TrackModel]> audio-tracks: [];
    in-out property <[TrackModel]> subtitle-tracks: [];
    
    // Selection
    in-out property <int> selected-clip-id: -1;
    
    // Metadata
    in-out property <string> video-codec: "";
    in-out property <string> audio-codec: "";
    in-out property <string> project-path: "";
    in-out property <bool> is-saved: true;
    
    // Callbacks to Rust backend
    callback play-pause();
    callback seek(int);  // position in ms
    callback set-volume(float);
    callback toggle-mute();
    callback toggle-loop();
    callback set-speed(string);
    callback add-video-track();
    callback add-audio-track();
    callback add-subtitle-track();
    callback select-clip(int);
    callback split-clip-at-playhead();
    callback delete-selected-clip();
    callback rotate-clip(int);  // degrees: 90, -90, 180
    callback flip-clip(string); // "horizontal" or "vertical"
    callback enter-trim-mode();
    callback apply-trim(int, int);  // start-ms, end-ms
    callback cancel-trim();
    callback export-video();
    callback open-project();
    callback save-project();
    
    VerticalLayout {
        spacing: 0px;
        
        // Export progress bar (only visible during export)
        if is-exporting: ExportProgress {
            progress: export-progress;
        }
        
        // Main editor view (hidden during trim mode)
        if !trim-mode: VerticalLayout {
            spacing: 0px;
            
            // Video Preview Area
            VideoPreview {
                frame: current-frame;
                is-playing: is-playing;
                vertical-stretch: 1;
                
                play-pause => { root.play-pause(); }
            }
            
            // Transport Controls
            TransportBar {
                current-time-ms: current-time-ms;
                total-duration-ms: total-duration-ms;
                is-playing: is-playing;
                is-looping: is-looping;
                is-muted: is-muted;
                volume: volume;
                playback-speed: playback-speed;
                
                play-pause => { root.play-pause(); }
                seek(pos) => { root.seek(pos); }
                volume-changed(v) => { root.set-volume(v); }
                toggle-mute => { root.toggle-mute(); }
                toggle-loop => { root.toggle-loop(); }
                speed-changed(s) => { root.set-speed(s); }
            }
            
            // Multi-track Timeline
            Timeline {
                video-tracks: video-tracks;
                audio-tracks: audio-tracks;
                subtitle-tracks: subtitle-tracks;
                playhead-position-ms: current-time-ms;
                total-duration-ms: total-duration-ms;
                selected-clip-id: selected-clip-id;
                
                add-video-track => { root.add-video-track(); }
                add-audio-track => { root.add-audio-track(); }
                add-subtitle-track => { root.add-subtitle-track(); }
                clip-selected(id) => { root.select-clip(id); }
                playhead-dragged(pos) => { root.seek(pos); }
            }
            
            // Status Footer
            StatusFooter {
                video-codec: video-codec;
                audio-codec: audio-codec;
                project-path: project-path;
                is-saved: is-saved;
            }
        }
        
        // Trim Mode (replaces main view when active)
        if trim-mode: TrimMode {
            frame: current-frame;
            duration-ms: total-duration-ms;
            
            apply-trim(start, end) => { root.apply-trim(start, end); }
            cancel-trim => { root.cancel-trim(); }
        }
    }
    
    // Keyboard shortcuts via FocusScope
    forward-focus: keyboard-handler;
    
    keyboard-handler := FocusScope {
        key-pressed(event) => {
            if event.text == " " {
                root.play-pause();
                return accept;
            }
            if event.text == "j" {
                // Shuttle backward
                return accept;
            }
            if event.text == "k" {
                root.play-pause();
                return accept;
            }
            if event.text == "l" {
                // Shuttle forward
                return accept;
            }
            if event.modifiers.meta && event.text == "t" {
                root.enter-trim-mode();
                return accept;
            }
            if event.modifiers.meta && event.text == "s" {
                root.save-project();
                return accept;
            }
            return reject;
        }
    }
}
```

### 2. Video Preview with Floating Draggable Controls (`ui/components/video-preview.slint`)

This implements QuickTime-style floating controls that:
- Are 60% width, centered horizontally, positioned at bottom 10%
- Auto-hide after 5 seconds of no mouse/keyboard activity
- Can be dragged anywhere within the video area (position persists)
- Feature a compact two-row layout

```slint
import { Theme } from "../theme.slint";

export component FloatingControls inherits Rectangle {
    in property <bool> is-playing: false;
    in property <bool> is-playing-backward: false;
    in property <int> current-time-ms: 0;
    in property <int> total-duration-ms: 0;
    in property <float> volume: 1.0;
    in property <bool> is-muted: false;
    in property <bool> is-looping: false;
    in property <string> playback-speed: "1×";
    in property <bool> tools-panel-open: false;
    
    in-out property <length> drag-offset-x: 0px;
    in-out property <length> drag-offset-y: 0px;
    
    callback play-pause();
    callback play-backward();
    callback play-forward();
    callback skip-back();
    callback skip-forward();
    callback seek(int);
    callback volume-changed(float);
    callback toggle-mute();
    callback toggle-loop();
    callback speed-changed(string);
    callback toggle-fullscreen();
    callback toggle-tools-panel();
    
    // Tool callbacks
    callback tool-magic-wand();
    callback tool-lasso();
    callback tool-crop();
    callback tool-contrast();
    callback tool-lighting();
    callback effect-face-swap();
    callback effect-face-enhance();
    callback effect-remove-bg();
    
    width: 60%;
    max-width: 640px;
    border-radius: 16px;
    background: rgba(0, 0, 0, 0.5);
    
    // Backdrop blur (platform-dependent)
    // backdrop-filter: blur(12px);
    
    VerticalLayout {
        padding: 16px;
        padding-top: 8px;
        spacing: 8px;
        
        // ROW 1: Transport + Secondary Controls
        HorizontalLayout {
            alignment: space-between;
            
            // Left: Drag handle + Skip back
            HorizontalLayout {
                spacing: 8px;
                
                // Drag handle
                Rectangle {
                    width: 24px;
                    height: 24px;
                    border-radius: 4px;
                    background: drag-touch.has-hover ? rgba(255,255,255,0.1) : transparent;
                    
                    // Grip icon (horizontal lines)
                    VerticalLayout {
                        alignment: center;
                        spacing: 2px;
                        for i in 2: Rectangle {
                            width: 12px;
                            height: 2px;
                            border-radius: 1px;
                            background: rgba(255, 255, 255, 0.4);
                        }
                    }
                    
                    drag-touch := TouchArea {
                        mouse-cursor: grab;
                        // Drag handling - update drag-offset-x/y in Rust
                    }
                }
                
                // Skip back button
                IconButton {
                    icon: @image-url("../icons/skip-back.svg");
                    size: 18px;
                    clicked => { root.skip-back(); }
                }
            }
            
            // Center: Playback direction + Play/Pause
            HorizontalLayout {
                spacing: 12px;
                alignment: center;
                
                // Backward play
                IconButton {
                    icon: @image-url("../icons/arrow-left.svg");
                    size: 16px;
                    active: is-playing-backward;
                    clicked => { root.play-backward(); }
                }
                
                // Main Play/Pause button
                Rectangle {
                    width: 40px;
                    height: 40px;
                    border-radius: 20px;
                    background: rgba(255, 255, 255, 0.2);
                    
                    animate background { duration: 150ms; }
                    
                    states [
                        hovered when play-touch.has-hover: {
                            background: rgba(255, 255, 255, 0.3);
                        }
                    ]
                    
                    Image {
                        source: is-playing 
                            ? @image-url("../icons/pause.svg")
                            : @image-url("../icons/play.svg");
                        width: 20px;
                        height: 20px;
                        x: (parent.width - self.width) / 2 + (is-playing ? 0 : 2px);
                        y: (parent.height - self.height) / 2;
                        colorize: white;
                    }
                    
                    play-touch := TouchArea {
                        clicked => { root.play-pause(); }
                    }
                }
                
                // Forward play
                IconButton {
                    icon: @image-url("../icons/arrow-right.svg");
                    size: 16px;
                    active: !is-playing-backward && is-playing;
                    clicked => { root.play-forward(); }
                }
            }
            
            // Right: Skip forward + Speed + Loop + Volume + Fullscreen + Tools
            HorizontalLayout {
                spacing: 8px;
                
                IconButton {
                    icon: @image-url("../icons/skip-forward.svg");
                    size: 18px;
                    clicked => { root.skip-forward(); }
                }
                
                // Divider
                Rectangle { width: 1px; height: 16px; background: rgba(255,255,255,0.2); }
                
                // Speed dropdown
                SpeedDropdown {
                    current-speed: playback-speed;
                    speed-selected(s) => { root.speed-changed(s); }
                }
                
                // Loop toggle
                IconButton {
                    icon: @image-url("../icons/repeat.svg");
                    size: 14px;
                    active: is-looping;
                    clicked => { root.toggle-loop(); }
                }
                
                // Volume
                HorizontalLayout {
                    spacing: 4px;
                    
                    IconButton {
                        icon: is-muted 
                            ? @image-url("../icons/volume-x.svg")
                            : @image-url("../icons/volume-2.svg");
                        size: 14px;
                        clicked => { root.toggle-mute(); }
                    }
                    
                    Slider {
                        width: 48px;
                        minimum: 0;
                        maximum: 100;
                        value: is-muted ? 0 : volume * 100;
                        changed(v) => { root.volume-changed(v / 100); }
                    }
                }
                
                // Fullscreen
                IconButton {
                    icon: @image-url("../icons/maximize.svg");
                    size: 14px;
                    clicked => { root.toggle-fullscreen(); }
                }
                
                // Divider
                Rectangle { width: 1px; height: 16px; background: rgba(255,255,255,0.2); }
                
                // Tools panel toggle (double chevron)
                Rectangle {
                    width: 24px;
                    height: 24px;
                    border-radius: 4px;
                    background: tools-panel-open 
                        ? rgba(10, 132, 255, 0.3) 
                        : (tools-touch.has-hover ? rgba(255,255,255,0.1) : transparent);
                    
                    Image {
                        source: @image-url("../icons/chevrons-right.svg");
                        width: 14px;
                        height: 14px;
                        x: (parent.width - self.width) / 2;
                        y: (parent.height - self.height) / 2;
                        colorize: tools-panel-open ? Theme.accent-blue : rgba(255,255,255,0.7);
                    }
                    
                    tools-touch := TouchArea {
                        clicked => { root.toggle-tools-panel(); }
                    }
                }
            }
        }
        
        // ROW 2: Scrub bar with time on sides
        HorizontalLayout {
            spacing: 12px;
            
            // Current time (left)
            Text {
                text: format-time(current-time-ms);
                color: white;
                font-size: 11px;
                font-family: "SF Mono";
                min-width: 56px;
                horizontal-alignment: right;
            }
            
            // Scrub bar
            Rectangle {
                horizontal-stretch: 1;
                height: 6px;
                border-radius: 3px;
                background: rgba(255, 255, 255, 0.2);
                
                // Progress fill
                Rectangle {
                    width: total-duration-ms > 0 
                        ? (current-time-ms / total-duration-ms) * parent.width 
                        : 0;
                    height: 100%;
                    border-radius: 3px;
                    background: white;
                }
                
                // Playhead knob
                Rectangle {
                    x: total-duration-ms > 0 
                        ? (current-time-ms / total-duration-ms) * parent.width - 6px
                        : -6px;
                    y: -3px;
                    width: 12px;
                    height: 12px;
                    border-radius: 6px;
                    background: white;
                    drop-shadow-blur: 4px;
                    drop-shadow-color: rgba(0,0,0,0.3);
                }
                
                TouchArea {
                    clicked(e) => {
                        root.seek((e.x / self.width) * total-duration-ms);
                    }
                }
            }
            
            // Total duration (right)
            Text {
                text: format-time(total-duration-ms);
                color: rgba(255, 255, 255, 0.6);
                font-size: 11px;
                font-family: "SF Mono";
                min-width: 56px;
            }
        }
    }
    
    // Tools popup panel (appears above the chevron button)
    if tools-panel-open: Rectangle {
        x: parent.width - 180px;
        y: -220px;
        width: 160px;
        border-radius: 8px;
        background: Theme.background-secondary;
        drop-shadow-blur: 12px;
        drop-shadow-color: rgba(0, 0, 0, 0.4);
        
        VerticalLayout {
            padding: 8px;
            spacing: 2px;
            
            // Edit tools
            ToolMenuItem { icon: "wand"; label: "Magic Wand"; clicked => { root.tool-magic-wand(); } }
            ToolMenuItem { icon: "lasso"; label: "Lasso Select"; clicked => { root.tool-lasso(); } }
            ToolMenuItem { icon: "crop"; label: "Crop"; clicked => { root.tool-crop(); } }
            
            Rectangle { height: 1px; background: Theme.border-subtle; }
            
            ToolMenuItem { icon: "contrast"; label: "Contrast"; clicked => { root.tool-contrast(); } }
            ToolMenuItem { icon: "sun"; label: "Lighting"; clicked => { root.tool-lighting(); } }
            
            Rectangle { height: 1px; background: Theme.border-subtle; }
            
            // Effects submenu
            ToolMenuSubmenu {
                label: "Effects";
                
                ToolMenuItem { icon: "user"; label: "Face Swap"; clicked => { root.effect-face-swap(); } }
                ToolMenuItem { icon: "wand"; label: "Face Enhance"; clicked => { root.effect-face-enhance(); } }
                ToolMenuItem { icon: "eraser"; label: "Remove BG"; clicked => { root.effect-remove-bg(); } }
                Rectangle { height: 1px; background: Theme.border-subtle; }
                ToolMenuItem { icon: "sparkles"; label: "AI Upscale"; disabled: true; }
            }
        }
    }
}

export component VideoPreview inherits Rectangle {
    in property <image> frame;
    in property <bool> is-playing: false;
    in property <bool> is-playing-backward: false;
    in property <int> current-time-ms: 0;
    in property <int> total-duration-ms: 0;
    in property <float> volume: 1.0;
    in property <bool> is-muted: false;
    in property <bool> is-looping: false;
    in property <string> playback-speed: "1×";
    
    // Forward all callbacks
    callback play-pause();
    callback play-backward();
    callback play-forward();
    callback skip-back();
    callback skip-forward();
    callback seek(int);
    callback volume-changed(float);
    callback toggle-mute();
    callback toggle-loop();
    callback speed-changed(string);
    callback toggle-fullscreen();
    
    property <bool> controls-visible: true;
    property <bool> tools-panel-open: false;
    property <length> controls-offset-x: 0px;
    property <length> controls-offset-y: 0px;
    
    background: Theme.surface-preview;
    
    // Video frame
    Image {
        source: frame;
        image-fit: contain;
        width: 100%;
        height: 100%;
    }
    
    // Floating controls - positioned at bottom 10%, centered
    FloatingControls {
        x: (parent.width - self.width) / 2 + controls-offset-x;
        y: parent.height * 0.9 - self.height + controls-offset-y;
        opacity: controls-visible ? 1.0 : 0.0;
        
        animate opacity { duration: 300ms; easing: ease-out; }
        
        is-playing: root.is-playing;
        is-playing-backward: root.is-playing-backward;
        current-time-ms: root.current-time-ms;
        total-duration-ms: root.total-duration-ms;
        volume: root.volume;
        is-muted: root.is-muted;
        is-looping: root.is-looping;
        playback-speed: root.playback-speed;
        tools-panel-open: tools-panel-open;
        
        play-pause => { root.play-pause(); }
        play-backward => { root.play-backward(); }
        play-forward => { root.play-forward(); }
        skip-back => { root.skip-back(); }
        skip-forward => { root.skip-forward(); }
        seek(pos) => { root.seek(pos); }
        volume-changed(v) => { root.volume-changed(v); }
        toggle-mute => { root.toggle-mute(); }
        toggle-loop => { root.toggle-loop(); }
        speed-changed(s) => { root.speed-changed(s); }
        toggle-fullscreen => { root.toggle-fullscreen(); }
        toggle-tools-panel => { tools-panel-open = !tools-panel-open; }
    }
    
    // Mouse movement detection - resets 5-second hide timer
    TouchArea {
        width: 100%;
        height: 100%;
        
        moved => {
            controls-visible = true;
            // In Rust: reset hide timer to 5 seconds
        }
        
        clicked => {
            root.play-pause();
        }
    }
}
```

### 3. Transport Bar Component (`ui/components/transport-bar.slint`)

```slint
import { Theme } from "../theme.slint";
import { Slider } from "std-widgets.slint";

component IconButton inherits Rectangle {
    in property <image> icon;
    in property <bool> active: false;
    in property <color> active-color: Theme.accent-blue;
    callback clicked();
    
    width: 28px;
    height: 28px;
    border-radius: 4px;
    background: touch.has-hover ? Theme.background-hover : transparent;
    
    Image {
        source: icon;
        width: 16px;
        height: 16px;
        x: (parent.width - self.width) / 2;
        y: (parent.height - self.height) / 2;
        colorize: active ? active-color : Theme.text-secondary;
    }
    
    touch := TouchArea {
        clicked => { root.clicked(); }
    }
}

component Timecode inherits HorizontalLayout {
    in property <int> current-ms;
    in property <int> total-ms;
    
    spacing: 6px;
    
    // Helper: format milliseconds to HH:MM:SS.mmm
    pure function format-time(ms: int) -> string {
        // Implement in Rust via callback or compute here
        // For now, placeholder
        return "00:00:00.000";
    }
    
    Text {
        text: format-time(current-ms);
        color: Theme.text-primary;
        font-size: 12px;
        font-family: "SF Mono";
    }
    
    Text {
        text: "/";
        color: Theme.text-tertiary;
        font-size: 12px;
    }
    
    Text {
        text: format-time(total-ms);
        color: Theme.text-secondary;
        font-size: 12px;
        font-family: "SF Mono";
    }
}

component SpeedDropdown inherits Rectangle {
    in-out property <string> current-speed: "1×";
    in property <bool> menu-open: false;
    callback speed-selected(string);
    
    width: 50px;
    height: 24px;
    border-radius: 4px;
    background: touch.has-hover || menu-open ? Theme.background-hover : Theme.background-tertiary;
    
    HorizontalLayout {
        alignment: center;
        spacing: 4px;
        
        Text {
            text: current-speed;
            color: Theme.text-primary;
            font-size: 11px;
        }
        
        // Dropdown chevron
        Path {
            viewbox-width: 10;
            viewbox-height: 10;
            commands: "M 2 3 L 5 7 L 8 3";
            stroke: Theme.text-secondary;
            stroke-width: 1.5px;
            width: 10px;
            height: 10px;
        }
    }
    
    touch := TouchArea {
        clicked => {
            // Toggle menu in Rust
        }
    }
    
    // Popup menu (implement with PopupWindow)
    if menu-open: PopupWindow {
        x: 0;
        y: parent.height + 4px;
        width: 60px;
        
        Rectangle {
            background: Theme.background-secondary;
            border-radius: 6px;
            drop-shadow-blur: 10px;
            drop-shadow-color: rgba(0,0,0,0.3);
            
            VerticalLayout {
                padding: 4px;
                
                for speed in ["0.5×", "0.75×", "1×", "1.25×", "1.5×", "2×"]: Rectangle {
                    height: 28px;
                    border-radius: 4px;
                    background: speed == current-speed ? Theme.accent-blue : 
                               (touch-item.has-hover ? Theme.background-hover : transparent);
                    
                    Text {
                        text: speed;
                        color: Theme.text-primary;
                        font-size: 12px;
                        horizontal-alignment: center;
                        vertical-alignment: center;
                    }
                    
                    touch-item := TouchArea {
                        clicked => { root.speed-selected(speed); }
                    }
                }
            }
        }
    }
}

component VolumeControl inherits HorizontalLayout {
    in-out property <float> value: 1.0;
    in property <bool> is-muted: false;
    callback volume-changed(float);
    callback toggle-mute();
    
    spacing: 8px;
    
    IconButton {
        icon: is-muted || value == 0 
            ? @image-url("../icons/volume-mute.svg")
            : @image-url("../icons/volume.svg");
        clicked => { root.toggle-mute(); }
    }
    
    Slider {
        width: 80px;
        minimum: 0;
        maximum: 100;
        value: is-muted ? 0 : value * 100;
        changed(v) => { root.volume-changed(v / 100); }
    }
}

export component TransportBar inherits Rectangle {
    in property <int> current-time-ms: 0;
    in property <int> total-duration-ms: 0;
    in property <bool> is-playing: false;
    in property <bool> is-looping: false;
    in property <bool> is-muted: false;
    in property <float> volume: 1.0;
    in property <string> playback-speed: "1×";
    
    callback play-pause();
    callback seek(int);
    callback volume-changed(float);
    callback toggle-mute();
    callback toggle-loop();
    callback speed-changed(string);
    
    height: 64px;
    background: Theme.surface-timeline;
    
    VerticalLayout {
        spacing: 4px;
        
        // Top row: Timecode and controls
        HorizontalLayout {
            height: 28px;
            padding-left: 16px;
            padding-right: 16px;
            alignment: space-between;
            
            Timecode {
                current-ms: current-time-ms;
                total-ms: total-duration-ms;
            }
            
            HorizontalLayout {
                spacing: 12px;
                
                SpeedDropdown {
                    current-speed: playback-speed;
                    speed-selected(s) => { root.speed-changed(s); }
                }
                
                IconButton {
                    icon: @image-url("../icons/loop.svg");
                    active: is-looping;
                    active-color: Theme.accent-blue;
                    clicked => { root.toggle-loop(); }
                }
                
                VolumeControl {
                    value: volume;
                    is-muted: is-muted;
                    volume-changed(v) => { root.volume-changed(v); }
                    toggle-mute => { root.toggle-mute(); }
                }
            }
        }
        
        // Bottom row: Playback controls and scrubber
        HorizontalLayout {
            height: 32px;
            padding-left: 12px;
            padding-right: 12px;
            spacing: 8px;
            
            // Play/Pause
            IconButton {
                icon: is-playing 
                    ? @image-url("../icons/pause.svg") 
                    : @image-url("../icons/play.svg");
                clicked => { root.play-pause(); }
            }
            
            // Skip backward
            IconButton {
                icon: @image-url("../icons/skip-back.svg");
                clicked => { root.seek(0); }
            }
            
            // Scrub slider
            Slider {
                horizontal-stretch: 1;
                minimum: 0;
                maximum: total-duration-ms;
                value: current-time-ms;
                changed(v) => { root.seek(Math.round(v)); }
            }
            
            // Skip forward
            IconButton {
                icon: @image-url("../icons/skip-forward.svg");
                clicked => { root.seek(total-duration-ms); }
            }
        }
    }
}
```

### 4. Timeline Component (`ui/components/timeline.slint`)

This is the most complex component - implements iMovie-style filmstrip clips with multi-track support.

```slint
import { Theme } from "../theme.slint";
import { ScrollView } from "std-widgets.slint";

// Import the structs from app.slint
import { ClipModel, TrackModel } from "../app.slint";

component AddTrackButton inherits Rectangle {
    callback clicked();
    
    width: 20px;
    height: 20px;
    border-radius: 4px;
    background: touch.has-hover ? Theme.background-hover : Theme.background-tertiary;
    
    Text {
        text: "+";
        color: Theme.text-secondary;
        font-size: 14px;
        horizontal-alignment: center;
        vertical-alignment: center;
    }
    
    touch := TouchArea {
        clicked => { root.clicked(); }
    }
}

component ClipThumbnail inherits Rectangle {
    in property <ClipModel> clip;
    in property <string> track-type: "video";
    in property <bool> selected: false;
    in property <int> total-duration-ms;
    
    callback clicked();
    
    // Calculate width based on clip duration relative to total
    width: total-duration-ms > 0 
        ? (clip.duration-ms / total-duration-ms) * parent.width 
        : 100px;
    height: track-type == "subtitle" ? 32px : 48px;
    border-radius: 4px;
    background: clip.color;
    border-width: selected ? 2px : 0px;
    border-color: Theme.accent-yellow;
    clip: true;
    
    // Filmstrip thumbnails (video only)
    if track-type == "video": HorizontalLayout {
        for thumb in clip.thumbnails: Image {
            source: thumb;
            width: 48px;
            image-fit: cover;
        }
    }
    
    // Waveform placeholder (audio)
    if track-type == "audio": Rectangle {
        // SVG waveform would be generated from audio data
        // For now, show a simple wave pattern
    }
    
    // Subtitle markers
    if track-type == "subtitle": HorizontalLayout {
        padding: 8px;
        spacing: 4px;
        alignment: start;
        
        for i in 8: Rectangle {
            width: 16px;
            height: 6px;
            border-radius: 2px;
            background: rgba(255, 255, 255, 0.5);
        }
    }
    
    // Clip name label
    Rectangle {
        y: parent.height - 16px;
        width: 100%;
        height: 16px;
        background: @linear-gradient(180deg, transparent 0%, rgba(0,0,0,0.6) 100%);
        
        Text {
            text: clip.name;
            color: Theme.text-primary;
            font-size: 9px;
            padding-left: 4px;
            vertical-alignment: center;
            overflow: elide;
        }
    }
    
    // Trim handles (visible when selected)
    if selected: Rectangle {
        x: 0;
        width: 6px;
        height: 100%;
        background: Theme.accent-yellow;
        border-radius: 3px 0 0 3px;
        
        // Grip texture
        VerticalLayout {
            alignment: center;
            spacing: 2px;
            
            for i in 3: Rectangle {
                width: 3px;
                height: 1px;
                background: rgba(0, 0, 0, 0.4);
            }
        }
        
        TouchArea { mouse-cursor: ew-resize; }
    }
    
    if selected: Rectangle {
        x: parent.width - 6px;
        width: 6px;
        height: 100%;
        background: Theme.accent-yellow;
        border-radius: 0 3px 3px 0;
        
        VerticalLayout {
            alignment: center;
            spacing: 2px;
            
            for i in 3: Rectangle {
                width: 3px;
                height: 1px;
                background: rgba(0, 0, 0, 0.4);
            }
        }
        
        TouchArea { mouse-cursor: ew-resize; }
    }
    
    TouchArea {
        clicked => { root.clicked(); }
    }
}

component TrackRow inherits Rectangle {
    in property <TrackModel> track;
    in property <int> selected-clip-id: -1;
    in property <int> total-duration-ms;
    in property <bool> hovered: false;
    
    callback clip-selected(int);
    callback delete-track(int);
    
    height: track.track-type == "subtitle" ? 40px : 56px;
    background: Theme.surface-timeline;
    border-radius: 4px;
    
    HorizontalLayout {
        spacing: 0px;
        
        // Track label (fixed width) with delete button on hover
        Rectangle {
            width: 96px;
            background: Theme.background-secondary;
            border-radius: 4px 0 0 4px;
            
            HorizontalLayout {
                padding: 8px;
                spacing: 8px;
                
                // Track icon
                Rectangle {
                    width: 16px;
                    height: 16px;
                    
                    // Icon varies by track type
                    if track.track-type == "video": Image {
                        source: @image-url("../icons/film.svg");
                        colorize: Theme.clip-video;
                    }
                    if track.track-type == "audio": Image {
                        source: @image-url("../icons/music.svg");
                        colorize: Theme.clip-audio;
                    }
                    if track.track-type == "subtitle": Image {
                        source: @image-url("../icons/type.svg");
                        colorize: Theme.clip-subtitle;
                    }
                }
                
                Text {
                    text: track.name;
                    color: Theme.text-secondary;
                    font-size: 11px;
                    vertical-alignment: center;
                    overflow: elide;
                    horizontal-stretch: 1;
                }
            }
            
            // Delete button (visible on hover)
            if hovered: Rectangle {
                x: parent.width - 20px;
                y: (parent.height - 16px) / 2;
                width: 16px;
                height: 16px;
                border-radius: 4px;
                background: delete-touch.has-hover ? rgba(255, 69, 58, 0.2) : transparent;
                
                Image {
                    source: @image-url("../icons/trash-2.svg");
                    width: 12px;
                    height: 12px;
                    x: 2px;
                    y: 2px;
                    colorize: Theme.accent-red;
                }
                
                delete-touch := TouchArea {
                    clicked => { root.delete-track(track.id); }
                }
            }
            
            TouchArea {
                moved => { root.hovered = true; }
            }
        }
        
        // Clips area
        Rectangle {
            horizontal-stretch: 1;
            background: transparent;
            clip: true;
            
            if track.clips.length == 0: Text {
                text: "Drop clips here";
                color: Theme.text-tertiary;
                font-size: 10px;
                horizontal-alignment: center;
                vertical-alignment: center;
            }
            
            HorizontalLayout {
                padding: 4px;
                spacing: 2px;
                
                for clip in track.clips: ClipThumbnail {
                    clip: clip;
                    track-type: track.track-type;
                    selected: clip.id == selected-clip-id;
                    total-duration-ms: root.total-duration-ms;
                    
                    clicked => { root.clip-selected(clip.id); }
                }
            }
        }
    }
}

export component Timeline inherits Rectangle {
    in property <[TrackModel]> video-tracks: [];
    in property <[TrackModel]> audio-tracks: [];
    in property <[TrackModel]> subtitle-tracks: [];
    in property <int> playhead-position-ms: 0;
    in property <int> total-duration-ms: 0;
    in property <int> selected-clip-id: -1;
    in property <bool> show-video-tracks: true;
    in property <bool> show-audio-tracks: true;
    in property <bool> show-subtitle-tracks: false;  // Hidden by default
    
    callback add-video-track();
    callback add-audio-track();
    callback add-subtitle-track();
    callback delete-video-track(int);
    callback delete-audio-track(int);
    callback delete-subtitle-track(int);
    callback clip-selected(int);
    callback playhead-dragged(int);
    
    // Only render if at least one track type is visible
    visible: show-video-tracks || show-audio-tracks || show-subtitle-tracks;
    min-height: visible ? 160px : 0px;
    background: Theme.background-primary;
    
    ScrollView {
        VerticalLayout {
            // Increased padding for better readability
            padding-left: 16px;
            padding-right: 16px;
            padding-top: 12px;
            padding-bottom: 12px;
            spacing: 12px;
            
            // VIDEO TRACKS SECTION
            VerticalLayout {
                spacing: 4px;
                
                // Section header
                HorizontalLayout {
                    height: 20px;
                    
                    Text {
                        text: "VIDEO";
                        color: Theme.text-tertiary;
                        font-size: 10px;
                        font-weight: 600;
                        letter-spacing: 1px;
                    }
                    
                    Rectangle { horizontal-stretch: 1; }
                    
                    AddTrackButton {
                        clicked => { root.add-video-track(); }
                    }
                }
                
                // Video tracks
                for track in video-tracks: TrackRow {
                    track: track;
                    selected-clip-id: selected-clip-id;
                    total-duration-ms: total-duration-ms;
                    clip-selected(id) => { root.clip-selected(id); }
                }
            }
            
            // AUDIO TRACKS SECTION
            VerticalLayout {
                spacing: 4px;
                
                HorizontalLayout {
                    height: 20px;
                    
                    Text {
                        text: "AUDIO";
                        color: Theme.text-tertiary;
                        font-size: 10px;
                        font-weight: 600;
                        letter-spacing: 1px;
                    }
                    
                    Rectangle { horizontal-stretch: 1; }
                    
                    AddTrackButton {
                        clicked => { root.add-audio-track(); }
                    }
                }
                
                for track in audio-tracks: TrackRow {
                    track: track;
                    selected-clip-id: selected-clip-id;
                    total-duration-ms: total-duration-ms;
                    clip-selected(id) => { root.clip-selected(id); }
                }
            }
            
            // SUBTITLE TRACKS SECTION
            VerticalLayout {
                spacing: 4px;
                
                HorizontalLayout {
                    height: 20px;
                    
                    Text {
                        text: "SUBTITLES";
                        color: Theme.text-tertiary;
                        font-size: 10px;
                        font-weight: 600;
                        letter-spacing: 1px;
                    }
                    
                    Rectangle { horizontal-stretch: 1; }
                    
                    AddTrackButton {
                        clicked => { root.add-subtitle-track(); }
                    }
                }
                
                for track in subtitle-tracks: TrackRow {
                    track: track;
                    selected-clip-id: selected-clip-id;
                    total-duration-ms: total-duration-ms;
                    clip-selected(id) => { root.clip-selected(id); }
                }
            }
        }
        
        // Playhead overlay (spans all tracks)
        Rectangle {
            x: total-duration-ms > 0 
                ? (playhead-position-ms / total-duration-ms) * parent.width 
                : 0;
            width: 2px;
            height: 100%;
            background: Theme.text-primary;
            drop-shadow-blur: 4px;
            drop-shadow-color: rgba(255, 255, 255, 0.5);
            
            // Playhead top marker
            Rectangle {
                y: -4px;
                x: -4px;
                width: 10px;
                height: 8px;
                background: Theme.text-primary;
                
                // Triangle pointing down
                clip: true;
                Path {
                    viewbox-width: 10;
                    viewbox-height: 8;
                    commands: "M 0 0 L 10 0 L 5 8 Z";
                    fill: Theme.text-primary;
                }
            }
        }
    }
}
```

### 5. Status Footer (`ui/components/status-footer.slint`)

```slint
import { Theme } from "../theme.slint";

export component StatusFooter inherits Rectangle {
    in property <string> video-codec: "";
    in property <string> audio-codec: "";
    in property <string> project-path: "";
    in property <bool> is-saved: true;
    
    height: 24px;
    background: Theme.background-primary;
    border-width: 1px 0 0 0;
    border-color: Theme.border-subtle;
    
    HorizontalLayout {
        padding-left: 12px;
        padding-right: 12px;
        alignment: space-between;
        
        // Codec info
        HorizontalLayout {
            spacing: 8px;
            
            Text {
                text: video-codec != "" ? video-codec : "—";
                color: Theme.text-secondary;
                font-size: 11px;
                vertical-alignment: center;
            }
            
            Rectangle {
                width: 1px;
                height: 12px;
                background: Theme.border-subtle;
            }
            
            Text {
                text: audio-codec != "" ? audio-codec : "—";
                color: Theme.text-secondary;
                font-size: 11px;
                vertical-alignment: center;
            }
        }
        
        // Project path
        Text {
            text: project-path != "" ? project-path : "Untitled";
            color: project-path != "" ? Theme.text-secondary : Theme.text-tertiary;
            font-size: 11px;
            horizontal-alignment: center;
            vertical-alignment: center;
            overflow: elide;
            horizontal-stretch: 1;
        }
        
        // Save status
        HorizontalLayout {
            spacing: 6px;
            
            Rectangle {
                width: 8px;
                height: 8px;
                border-radius: 4px;
                background: is-saved ? Theme.accent-green : Theme.text-tertiary;
            }
            
            Text {
                text: is-saved ? "Saved" : "Unsaved";
                color: is-saved ? Theme.accent-green : Theme.text-secondary;
                font-size: 11px;
                vertical-alignment: center;
            }
        }
    }
}
```

### 6. Trim Mode (`ui/components/trim-mode.slint`)

QuickTime-signature yellow trim interface:

```slint
import { Theme } from "../theme.slint";
import { Button } from "std-widgets.slint";

export component TrimMode inherits Rectangle {
    in property <image> frame;
    in property <int> duration-ms;
    in-out property <int> trim-start-ms: 0;
    in-out property <int> trim-end-ms: duration-ms;
    
    callback apply-trim(int, int);
    callback cancel-trim();
    
    background: Theme.surface-preview;
    
    VerticalLayout {
        spacing: 0px;
        
        // Video preview
        Image {
            source: frame;
            image-fit: contain;
            vertical-stretch: 1;
        }
        
        // Trim controls bar
        Rectangle {
            height: 120px;
            background: Theme.background-primary;
            
            VerticalLayout {
                padding: 16px;
                spacing: 12px;
                
                // Filmstrip with trim handles
                Rectangle {
                    height: 56px;
                    background: Theme.background-secondary;
                    border-radius: 6px;
                    clip: true;
                    
                    // Thumbnail strip (would come from video frames)
                    HorizontalLayout {
                        // Thumbnails go here
                    }
                    
                    // Dimmed region before trim start
                    Rectangle {
                        x: 0;
                        width: duration-ms > 0 
                            ? (trim-start-ms / duration-ms) * parent.width 
                            : 0;
                        height: 100%;
                        background: rgba(0, 0, 0, 0.6);
                    }
                    
                    // Dimmed region after trim end
                    Rectangle {
                        x: duration-ms > 0 
                            ? (trim-end-ms / duration-ms) * parent.width 
                            : parent.width;
                        width: duration-ms > 0 
                            ? ((duration-ms - trim-end-ms) / duration-ms) * parent.width 
                            : 0;
                        height: 100%;
                        background: rgba(0, 0, 0, 0.6);
                    }
                    
                    // Yellow trim frame (QuickTime signature)
                    Rectangle {
                        x: duration-ms > 0 
                            ? (trim-start-ms / duration-ms) * parent.width 
                            : 0;
                        width: duration-ms > 0 
                            ? ((trim-end-ms - trim-start-ms) / duration-ms) * parent.width 
                            : parent.width;
                        height: 100%;
                        border-width: 3px;
                        border-color: Theme.accent-yellow;
                        border-radius: 4px;
                        background: transparent;
                        
                        // Left trim handle
                        Rectangle {
                            x: -10px;
                            width: 20px;
                            height: 100%;
                            background: Theme.accent-yellow;
                            border-radius: 4px 0 0 4px;
                            
                            // Grip lines
                            VerticalLayout {
                                alignment: center;
                                spacing: 3px;
                                
                                for i in 3: Rectangle {
                                    width: 6px;
                                    height: 1.5px;
                                    background: rgba(0, 0, 0, 0.4);
                                }
                            }
                            
                            TouchArea {
                                mouse-cursor: ew-resize;
                                // Drag handling in Rust
                            }
                        }
                        
                        // Right trim handle
                        Rectangle {
                            x: parent.width - 10px;
                            width: 20px;
                            height: 100%;
                            background: Theme.accent-yellow;
                            border-radius: 0 4px 4px 0;
                            
                            VerticalLayout {
                                alignment: center;
                                spacing: 3px;
                                
                                for i in 3: Rectangle {
                                    width: 6px;
                                    height: 1.5px;
                                    background: rgba(0, 0, 0, 0.4);
                                }
                            }
                            
                            TouchArea {
                                mouse-cursor: ew-resize;
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
                        clicked => { root.cancel-trim(); }
                    }
                    
                    Button {
                        text: "Trim";
                        primary: true;
                        clicked => { 
                            root.apply-trim(trim-start-ms, trim-end-ms); 
                        }
                    }
                }
            }
        }
    }
}
```

---

## Menu Structure

Implement the following menu bar structure based on the phases-ui.md specification:

### File Menu
- Open... (Ctrl+O)
- Open Recent → [submenu with recent files] + Clear Recent
- ---
- Close (Ctrl+W)
- Revert
- ---
- New Window
- Save (Ctrl+S)
- ---
- Insert Video... (Ctrl+I)
- Insert Audio... (Ctrl+Shift+I)
- ---
- New Video Track (Ctrl+Shift+N)
- New Audio Track (Ctrl+Shift+A)
- New Subtitle Track
- ---
- Export... (Ctrl+E)
- Cancel Export (Esc) [when exporting]

### Edit Menu
- Undo (Ctrl+Z)
- Redo (Ctrl+Shift+Z)
- ---
- Split Clip at Playhead (Ctrl+B)
- ---
- Move Clip to Track Below (Ctrl+Shift+Down)
- Move Clip to Track Above (Ctrl+Shift+Up)
- ---
- Rotate 90° Right (Ctrl+R)
- Rotate 90° Left (Ctrl+Shift+R)
- Flip Horizontal
- Flip Vertical
- ---
- Resize Video...
- ---
- Audio → [submenu: Remove Audio, Replace Audio..., Overlay Audio...]

### View Menu
- Loop Playback (Ctrl+L) [toggle]
- ---
- Zoom In (Ctrl+=)
- Zoom Out (Ctrl+-)
- Zoom to Fit (Ctrl+0)
- Actual Size (1:1)
- ---
- Enter/Exit Fullscreen (Esc to exit)
- ---
- Video Tracks [toggle]
- Audio Tracks [toggle]
- Subtitle Tracks [toggle]
- ---
- Status Bar [toggle]

### Effects Menu
- Face Swap (FaceFusion)
- Face Enhance
- Remove Background (RVM)
- ---
- AI Upscale... [disabled/roadmap]

### Window Menu
- Always on Top [toggle]
- ---
- Fit
- Fill
- Center

### Help Menu
- Overview (F1)
- Features
- Keyboard Shortcuts
- ---
- Media Formats & Tracks
- Supported Formats
- ---
- CLI Reference
- External AI & Tools
- ---
- Developers
- Agents
- UI Phases

---

## File Structure

Create the following directory structure:

```
ui/
├── app.slint                      # Main application window
├── theme.slint                    # Color and style tokens
├── components/
│   ├── video-preview.slint        # Video playback area
│   ├── transport-bar.slint        # Playback controls
│   ├── timeline.slint             # Multi-track timeline
│   ├── status-footer.slint        # Bottom status bar
│   ├── trim-mode.slint            # QuickTime-style trim view
│   └── export-progress.slint      # Export progress bar
└── icons/
    ├── play.svg
    ├── pause.svg
    ├── skip-back.svg
    ├── skip-forward.svg
    ├── loop.svg
    ├── volume.svg
    ├── volume-mute.svg
    ├── film.svg
    ├── music.svg
    ├── type.svg
    └── ... (other icons)
```

---

## Rust Integration Points

In your `main.rs` or app module, connect the Slint UI to your Rust backend:

```rust
slint::include_modules!();

fn main() {
    let app = ReelApp::new().unwrap();
    
    // Set up callbacks
    let app_weak = app.as_weak();
    app.on_play_pause(move || {
        // Toggle playback state
    });
    
    let app_weak = app.as_weak();
    app.on_seek(move |position_ms| {
        // Seek to position
    });
    
    let app_weak = app.as_weak();
    app.on_add_video_track(move || {
        let app = app_weak.upgrade().unwrap();
        let mut tracks = app.get_video_tracks();
        let new_id = tracks.row_count() as i32 + 1;
        // Add new track to model
        app.set_is_saved(false);
    });
    
    // Similar for other callbacks...
    
    app.run().unwrap();
}
```

---

## Key Implementation Notes

1. **Multi-track support**: The timeline supports multiple Video, Audio, and Subtitle tracks. Each section has a `+` button to add more tracks. **Subtitle tracks are hidden by default** - users enable them via View menu.

2. **Track deletion**: Each track row shows a red delete button (trash icon) on hover in the track label area. Clicking deletes the track.

3. **Floating draggable controls**: 
   - 60% width, max 640px, centered at bottom 10% of video area
   - Drag handle (grip icon) allows repositioning anywhere within video bounds
   - Position persists until changed by user
   - Auto-hide after **5 seconds** of no mouse/keyboard activity
   - Two-row compact layout: transport controls on top, scrub bar with flanking timestamps below

4. **Forward/Backward playback**: Arrow buttons indicate playback direction. Active direction shows blue highlight.

5. **Tools panel**: Double chevron (`>>`) button on far right opens a popup with:
   - Edit tools: Magic Wand, Lasso Select, Crop, Contrast, Lighting
   - Effects submenu: Face Swap, Face Enhance, Remove BG, AI Upscale (disabled)

6. **QuickTime trim handles**: Use the signature yellow color (`#FFD60A`) for trim handles with grip texture.

7. **Timeline padding**: Use `px-4 py-3` (16px horizontal, 12px vertical) for comfortable spacing from window edges.

8. **Video fills available space**: When all tracks hidden (via View menu toggles), video preview extends to footer/bottom.

9. **Playhead**: A white vertical line with glow effect (`drop-shadow`) that spans all tracks.

10. **Keyboard shortcuts**: Implement via `FocusScope` - Space for play/pause, J/K/L for shuttle, Cmd+T for trim mode.

11. **Animations**: Use 150-200ms durations with `ease-out` for most transitions. Controls fade uses 300ms.

12. **Empty states**: Show "Drop clips here" placeholder text in empty tracks.

13. **Clip selection**: Selected clips show yellow border (`border-color: #FFD60A`) and reveal trim handles.

---

## Testing Checklist

### Core Functionality
- [ ] App launches with correct dark theme
- [ ] Video preview displays frames correctly
- [ ] Play/pause toggles work (click and keyboard)
- [ ] Scrub slider seeks video
- [ ] Volume control and mute work
- [ ] Loop toggle persists state
- [ ] Speed dropdown shows options and changes playback
- [ ] Forward/backward playback indicators highlight correctly

### Floating Controls
- [ ] Controls appear at 60% width, centered near bottom
- [ ] Controls auto-hide after 5 seconds of inactivity
- [ ] Controls reappear on mouse movement
- [ ] Drag handle allows repositioning controls
- [ ] Dragged position persists until changed
- [ ] Tools panel (>>) opens popup with edit tools
- [ ] Effects submenu expands from tools panel
- [ ] Current time shows on left of scrub bar
- [ ] Total duration shows on right of scrub bar

### Timeline
- [ ] Timeline displays video and audio tracks (subtitles hidden by default)
- [ ] `+` buttons add new tracks
- [ ] Delete button appears on track label hover
- [ ] Clicking delete removes the track
- [ ] Timeline has comfortable padding (px-4 py-3)
- [ ] Clicking clips selects them (yellow border appears)
- [ ] Trim handles appear on selected clips
- [ ] Playhead moves with current time

### View Menu Toggles
- [ ] Video Tracks toggle shows/hides video section
- [ ] Audio Tracks toggle shows/hides audio section
- [ ] Subtitle Tracks toggle shows/hides subtitle section
- [ ] Status Bar toggle shows/hides footer
- [ ] When all tracks hidden, video fills to footer/bottom

### Other
- [ ] Status footer shows codec info and save state
- [ ] Trim mode activates and shows yellow trim bar
- [ ] Export progress bar appears during export
- [ ] All keyboard shortcuts work
- [ ] All menus open and display correctly
