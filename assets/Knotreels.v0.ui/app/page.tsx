"use client"

import { useState, useEffect, useCallback, useRef, useMemo } from "react"
import {
  Play,
  Pause,
  SkipBack,
  SkipForward,
  Repeat,
  Volume2,
  VolumeX,
  Maximize,
  Minimize,
  ChevronDown,
  ChevronRight,
  Film,
  Music,
  Check,
  Circle,
  Scissors,
  RotateCw,
  RotateCcw,
  FlipHorizontal2,
  FlipVertical2,
  X,
  GripVertical,
  Plus,
  Type,
  FolderOpen,
  Save,
  Download,
  FileVideo,
  FileAudio,
  Undo2,
  Redo2,
  Eye,
  EyeOff,
  ZoomIn,
  ZoomOut,
  Sparkles,
  PanelTop,
  AppWindow,
  HelpCircle,
  Clock,
  Trash2,
  ArrowUp,
  ArrowDown,
  ArrowLeft,
  ArrowRight,
  Move,
  Wand2,
  User,
  Eraser,
  GripHorizontal,
  ChevronsRight,
  Crop,
  Sun,
  Contrast,
  Lasso,
  Layers,
} from "lucide-react"

// Types for tracks
type Clip = { id: number; name: string; duration: number; color: string; offset?: number }
type Track = { id: number; name: string; clips: Clip[] }

// Initial mock data for tracks
const initialVideoTracks: Track[] = [
  {
    id: 1,
    name: "Video 1",
    clips: [
      { id: 1, name: "Intro.mp4", duration: 5000, color: "#4A90D9" },
      { id: 2, name: "Main_Content.mov", duration: 15000, color: "#5E9EE0" },
      { id: 3, name: "Outro.mp4", duration: 3000, color: "#4A90D9" },
    ],
  },
]

const initialAudioTracks: Track[] = [
  {
    id: 1,
    name: "Audio 1",
    clips: [
      { id: 1, name: "Background_Music.mp3", duration: 20000, color: "#5AC8FA" },
      { id: 2, name: "Voiceover.wav", duration: 8000, color: "#6DD5FF" },
    ],
  },
]

const initialSubtitleTracks: Track[] = [
  {
    id: 1,
    name: "Subtitles 1",
    clips: [
      { id: 1, name: "English.srt", duration: 23000, color: "#FF9F0A", offset: 0 },
    ],
  },
]

function formatTime(ms: number): string {
  const totalSeconds = Math.floor(ms / 1000)
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  const milliseconds = Math.floor((ms % 1000) / 10)
  return `${minutes}:${seconds.toString().padStart(2, "0")}.${milliseconds.toString().padStart(2, "0")}`
}

// Menu Components
function MenuItem({ 
  icon, 
  label, 
  shortcut, 
  onClick, 
  disabled, 
  checked, 
  hasSubmenu 
}: { 
  icon?: React.ReactNode
  label: string
  shortcut?: string
  onClick?: () => void
  disabled?: boolean
  checked?: boolean
  hasSubmenu?: boolean
}) {
  return (
    <button
      onClick={(e) => { e.stopPropagation(); onClick?.() }}
      disabled={disabled}
      className={`flex w-full items-center gap-3 px-3 py-1.5 text-left text-xs transition-colors ${
        disabled 
          ? "cursor-not-allowed text-[#6E6E6E]" 
          : "text-white hover:bg-[#0A84FF]"
      }`}
    >
      <span className="w-4 flex-shrink-0">
        {checked !== undefined ? (
          checked ? <Check size={14} className="text-[#0A84FF]" /> : null
        ) : icon}
      </span>
      <span className="flex-1">{label}</span>
      {shortcut && <span className="text-[#8E8E93]">{shortcut}</span>}
      {hasSubmenu && <ChevronRight size={12} className="text-[#8E8E93]" />}
    </button>
  )
}

function MenuDivider() {
  return <div className="my-1 h-px bg-[#4A4A4A]" />
}

export default function ReelMockup() {
  const [isPlaying, setIsPlaying] = useState(false)
  const [currentTime, setCurrentTime] = useState(0)
  const [isPlayingBackward, setIsPlayingBackward] = useState(false)
  const [volume, setVolume] = useState(0.75)
  const [isMuted, setIsMuted] = useState(false)
  const [isLooping, setIsLooping] = useState(false)
  const [playbackSpeed, setPlaybackSpeed] = useState("1×")
  const [showSpeedMenu, setShowSpeedMenu] = useState(false)
  const [isFullscreen, setIsFullscreen] = useState(false)
  const [controlsVisible, setControlsVisible] = useState(true)
  const [selectedClip, setSelectedClip] = useState<number | null>(null)
  const [isTrimMode, setIsTrimMode] = useState(false)
  const [trimStart, setTrimStart] = useState(20)
  const [trimEnd, setTrimEnd] = useState(80)
  const [isExporting, setIsExporting] = useState(false)
  const [exportProgress, setExportProgress] = useState(0)
  const [isSaved, setIsSaved] = useState(true)
  
  // Track state
  const [videoTracks, setVideoTracks] = useState<Track[]>(initialVideoTracks)
  const [audioTracks, setAudioTracks] = useState<Track[]>(initialAudioTracks)
  const [subtitleTracks, setSubtitleTracks] = useState<Track[]>(initialSubtitleTracks)

  // Add track handlers
  const addVideoTrack = () => {
    const newId = Math.max(...videoTracks.map(t => t.id), 0) + 1
    setVideoTracks([...videoTracks, { id: newId, name: `Video ${newId}`, clips: [] }])
    setIsSaved(false)
  }

  const addAudioTrack = () => {
    const newId = Math.max(...audioTracks.map(t => t.id), 0) + 1
    setAudioTracks([...audioTracks, { id: newId, name: `Audio ${newId}`, clips: [] }])
    setIsSaved(false)
  }

  const addSubtitleTrack = () => {
    const newId = Math.max(...subtitleTracks.map(t => t.id), 0) + 1
    setSubtitleTracks([...subtitleTracks, { id: newId, name: `Subtitles ${newId}`, clips: [] }])
    setIsSaved(false)
  }

  // Delete track handlers
  const deleteVideoTrack = (trackId: number) => {
    setVideoTracks(videoTracks.filter(t => t.id !== trackId))
    setIsSaved(false)
  }

  const deleteAudioTrack = (trackId: number) => {
    setAudioTracks(audioTracks.filter(t => t.id !== trackId))
    setIsSaved(false)
  }

  const deleteSubtitleTrack = (trackId: number) => {
    setSubtitleTracks(subtitleTracks.filter(t => t.id !== trackId))
    setIsSaved(false)
  }

  // Hover state for tracks
  const [hoveredTrack, setHoveredTrack] = useState<string | null>(null)

  // Menu state
  const [activeMenu, setActiveMenu] = useState<string | null>(null)
  
  // Big Buck Bunny test videos
  const recentFiles = useMemo(() => [
    { name: "Big Buck Bunny (1080p)", url: "https://download.blender.org/peach/bigbuckbunny_movies/BigBuckBunny_320x180.mp4" },
    { name: "Big Buck Bunny (720p)", url: "https://download.blender.org/peach/bigbuckbunny_movies/big_buck_bunny_720p_h264.mov" },
    { name: "Big Buck Bunny (480p)", url: "https://download.blender.org/peach/bigbuckbunny_movies/big_buck_bunny_480p_h264.mov" },
  ], [])
  
  // Video source state
  const [videoSrc, setVideoSrc] = useState(recentFiles[0].url)
  const videoRef = useRef<HTMLVideoElement>(null)

  // View visibility toggles
  const [showVideoTracks, setShowVideoTracks] = useState(true)
  const [showAudioTracks, setShowAudioTracks] = useState(true)
  const [showSubtitleTracks, setShowSubtitleTracks] = useState(false)
  const [showStatus, setShowStatus] = useState(true)
  const [zoomLevel, setZoomLevel] = useState(100)
  const [alwaysOnTop, setAlwaysOnTop] = useState(false)
  const [showToolsPanel, setShowToolsPanel] = useState(false)
  const [showEffectsSubmenu, setShowEffectsSubmenu] = useState(false)

  // Close menu when clicking outside
  useEffect(() => {
    const handleClickOutside = () => setActiveMenu(null)
    if (activeMenu) {
      document.addEventListener("click", handleClickOutside)
      return () => document.removeEventListener("click", handleClickOutside)
    }
  }, [activeMenu])

  const totalDuration = 23000 // 23 seconds total
  const timelineRef = useRef<HTMLDivElement>(null)
  const controlsTimeoutRef = useRef<NodeJS.Timeout>()
  const videoContainerRef = useRef<HTMLDivElement>(null)
  
  // Draggable controls state
  const [controlsPosition, setControlsPosition] = useState({ x: 0, y: 0 })
  const [isDragging, setIsDragging] = useState(false)
  const dragStartRef = useRef({ x: 0, y: 0, startX: 0, startY: 0 })

  // Simulate playback
  useEffect(() => {
    let interval: NodeJS.Timeout
    if (isPlaying) {
      interval = setInterval(() => {
        setCurrentTime((prev) => {
          const newTime = prev + 100
          if (newTime >= totalDuration) {
            if (isLooping) return 0
            setIsPlaying(false)
            return totalDuration
          }
          return newTime
        })
      }, 100)
    }
    return () => clearInterval(interval)
  }, [isPlaying, isLooping, totalDuration])

  // Auto-hide controls after 5 seconds of inactivity
  const showControls = useCallback(() => {
    setControlsVisible(true)
    if (controlsTimeoutRef.current) {
      clearTimeout(controlsTimeoutRef.current)
    }
    controlsTimeoutRef.current = setTimeout(() => {
      if (!isDragging) setControlsVisible(false)
    }, 5000)
  }, [isDragging])

  // Handle drag start for floating controls
  const handleDragStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
    setIsDragging(true)
    dragStartRef.current = {
      x: controlsPosition.x,
      y: controlsPosition.y,
      startX: e.clientX,
      startY: e.clientY,
    }
  }, [controlsPosition])

  // Handle drag move
  useEffect(() => {
    if (!isDragging) return

    const handleMouseMove = (e: MouseEvent) => {
      const deltaX = e.clientX - dragStartRef.current.startX
      const deltaY = e.clientY - dragStartRef.current.startY
      
      // Get container bounds
      const container = videoContainerRef.current
      if (container) {
        const rect = container.getBoundingClientRect()
        const controlsWidth = 320 // approx width of controls
        const controlsHeight = 80 // approx height of controls
        
        const newX = Math.max(-rect.width/2 + controlsWidth/2, Math.min(rect.width/2 - controlsWidth/2, dragStartRef.current.x + deltaX))
        const newY = Math.max(-rect.height/2 + controlsHeight/2, Math.min(rect.height/2 - controlsHeight/2, dragStartRef.current.y + deltaY))
        
        setControlsPosition({ x: newX, y: newY })
      }
      showControls() // Reset hide timer while dragging
    }

    const handleMouseUp = () => {
      setIsDragging(false)
    }

    document.addEventListener("mousemove", handleMouseMove)
    document.addEventListener("mouseup", handleMouseUp)

    return () => {
      document.removeEventListener("mousemove", handleMouseMove)
      document.removeEventListener("mouseup", handleMouseUp)
    }
  }, [isDragging, showControls])

  // Simulate export progress
  useEffect(() => {
    if (isExporting) {
      const interval = setInterval(() => {
        setExportProgress((prev) => {
          if (prev >= 100) {
            setIsExporting(false)
            return 0
          }
          return prev + 2
        })
      }, 100)
      return () => clearInterval(interval)
    }
  }, [isExporting])

  const handleSeek = (e: React.MouseEvent<HTMLDivElement>) => {
    const rect = e.currentTarget.getBoundingClientRect()
    const x = e.clientX - rect.left
    const percentage = x / rect.width
    setCurrentTime(Math.floor(percentage * totalDuration))
  }

  const playheadPosition = (currentTime / totalDuration) * 100

  // Speed options
  const speedOptions = ["0.25×", "0.5×", "0.75×", "1×", "1.25×", "1.5×", "2×"]

  if (isTrimMode) {
    return (
      <div className="flex h-screen flex-col bg-black">
        {/* Trim Mode Header */}
        <div className="flex items-center justify-between bg-[#1E1E1E] px-4 py-2">
          <span className="text-sm text-[#A0A0A0]">Trim Mode</span>
          <button onClick={() => setIsTrimMode(false)} className="text-[#A0A0A0] hover:text-white">
            <X size={20} />
          </button>
        </div>

        {/* Video Preview */}
        <div className="flex flex-1 items-center justify-center bg-black">
          <div className="flex h-3/4 w-3/4 items-center justify-center rounded-lg bg-[#1E1E1E]">
            <div className="text-center text-[#6E6E6E]">
              <Film size={64} className="mx-auto mb-4 opacity-50" />
              <p>Video Preview</p>
              <p className="text-sm">Frame at playhead position</p>
            </div>
          </div>
        </div>

        {/* QuickTime-Style Trim Bar */}
        <div className="bg-[#1E1E1E] p-4">
          {/* Filmstrip with trim handles */}
          <div className="relative mb-4 h-16 overflow-hidden rounded-lg bg-[#2D2D2D]">
            {/* Thumbnail strip simulation */}
            <div className="flex h-full">
              {Array.from({ length: 20 }).map((_, i) => (
                <div
                  key={i}
                  className="h-full flex-1 border-r border-[#3A3A3A]"
                  style={{
                    background: `linear-gradient(180deg, #4A90D9 0%, #3A7BC8 100%)`,
                    opacity: 0.8 + Math.random() * 0.2,
                  }}
                />
              ))}
            </div>

            {/* Dimmed area - before trim start */}
            <div
              className="absolute left-0 top-0 h-full bg-black/60"
              style={{ width: `${trimStart}%` }}
            />

            {/* Dimmed area - after trim end */}
            <div
              className="absolute right-0 top-0 h-full bg-black/60"
              style={{ width: `${100 - trimEnd}%` }}
            />

            {/* Yellow trim frame (QuickTime signature) */}
            <div
              className="absolute top-0 h-full border-[3px] border-[#FFD60A]"
              style={{
                left: `${trimStart}%`,
                width: `${trimEnd - trimStart}%`,
              }}
            >
              {/* Left handle */}
              <div
                className="absolute -left-3 top-0 flex h-full w-4 cursor-ew-resize flex-col items-center justify-center rounded-l bg-[#FFD60A]"
                onMouseDown={(e) => {
                  const startX = e.clientX
                  const startTrim = trimStart
                  const onMove = (e: MouseEvent) => {
                    const delta = ((e.clientX - startX) / (timelineRef.current?.offsetWidth || 1)) * 100
                    setTrimStart(Math.max(0, Math.min(trimEnd - 5, startTrim + delta)))
                  }
                  const onUp = () => {
                    document.removeEventListener("mousemove", onMove)
                    document.removeEventListener("mouseup", onUp)
                  }
                  document.addEventListener("mousemove", onMove)
                  document.addEventListener("mouseup", onUp)
                }}
              >
                <div className="space-y-1">
                  {[0, 1, 2].map((i) => (
                    <div key={i} className="h-[2px] w-2 bg-black/50" />
                  ))}
                </div>
              </div>

              {/* Right handle */}
              <div
                className="absolute -right-3 top-0 flex h-full w-4 cursor-ew-resize flex-col items-center justify-center rounded-r bg-[#FFD60A]"
                onMouseDown={(e) => {
                  const startX = e.clientX
                  const startTrim = trimEnd
                  const onMove = (e: MouseEvent) => {
                    const delta = ((e.clientX - startX) / (timelineRef.current?.offsetWidth || 1)) * 100
                    setTrimEnd(Math.min(100, Math.max(trimStart + 5, startTrim + delta)))
                  }
                  const onUp = () => {
                    document.removeEventListener("mousemove", onMove)
                    document.removeEventListener("mouseup", onUp)
                  }
                  document.addEventListener("mousemove", onMove)
                  document.addEventListener("mouseup", onUp)
                }}
              >
                <div className="space-y-1">
                  {[0, 1, 2].map((i) => (
                    <div key={i} className="h-[2px] w-2 bg-black/50" />
                  ))}
                </div>
              </div>
            </div>

            {/* Timeline reference */}
            <div ref={timelineRef} className="absolute inset-0" />
          </div>

          {/* Trim info and buttons */}
          <div className="flex items-center justify-between">
            <div className="text-sm text-[#A0A0A0]">
              <span className="font-mono">{formatTime(trimStart * totalDuration / 100)}</span>
              <span className="mx-2">→</span>
              <span className="font-mono">{formatTime(trimEnd * totalDuration / 100)}</span>
              <span className="ml-4 text-[#6E6E6E]">
                (Duration: {formatTime((trimEnd - trimStart) * totalDuration / 100)})
              </span>
            </div>
            <div className="flex gap-3">
              <button
                onClick={() => setIsTrimMode(false)}
                className="rounded-lg bg-[#3D3D3D] px-6 py-2 text-sm text-white transition-colors hover:bg-[#4A4A4A]"
              >
                Cancel
              </button>
              <button
                onClick={() => {
                  setIsTrimMode(false)
                  setIsSaved(false)
                }}
                className="rounded-lg bg-[#0A84FF] px-6 py-2 text-sm font-medium text-white transition-colors hover:bg-[#0077ED]"
              >
                Trim
              </button>
            </div>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div className="flex h-screen flex-col bg-[#1E1E1E]">
      {/* Export Progress Bar */}
      {isExporting && (
        <div className="h-1 bg-[#2D2D2D]">
          <div
            className="h-full bg-gradient-to-r from-[#0A84FF] to-[#5AC8FA] transition-all duration-100"
            style={{ width: `${exportProgress}%` }}
          />
        </div>
      )}

      {/* Menu Bar */}
      <div className="relative flex h-7 items-center bg-[#2D2D2D] px-2 text-xs text-[#A0A0A0]">
        <span className="mr-4 px-2 font-semibold text-white">Reel</span>
        
        {/* File Menu */}
        <div className="relative">
          <button
            onClick={(e) => { e.stopPropagation(); setActiveMenu(activeMenu === "file" ? null : "file") }}
            className={`px-3 py-1 ${activeMenu === "file" ? "bg-[#0A84FF] text-white" : "hover:bg-[#3D3D3D] hover:text-white"}`}
          >
            File
          </button>
          {activeMenu === "file" && (
            <div className="absolute left-0 top-full z-50 min-w-[220px] rounded-b-lg bg-[#3D3D3D] py-1 shadow-xl">
              <MenuItem icon={<FolderOpen size={14} />} label="Open..." shortcut="Ctrl+O" />
              <div className="group relative">
                <MenuItem icon={<Clock size={14} />} label="Open Recent" hasSubmenu />
                <div className="invisible absolute left-full top-0 min-w-[260px] rounded-lg bg-[#3D3D3D] py-1 shadow-xl group-hover:visible">
                  {recentFiles.map((file, i) => (
                    <MenuItem 
                      key={i} 
                      label={file.name} 
                      onClick={() => {
                        setVideoSrc(file.url)
                        setActiveMenu(null)
                        setCurrentTime(0)
                        setIsPlaying(false)
                      }}
                    />
                  ))}
                  <MenuDivider />
                  <MenuItem icon={<Trash2 size={14} />} label="Clear Recent" />
                </div>
              </div>
              <MenuDivider />
              <MenuItem icon={<X size={14} />} label="Close" shortcut="Ctrl+W" />
              <MenuItem icon={<Undo2 size={14} />} label="Revert" />
              <MenuDivider />
              <MenuItem icon={<AppWindow size={14} />} label="New Window" />
              <MenuItem icon={<Save size={14} />} label="Save" shortcut="Ctrl+S" />
              <MenuDivider />
              <MenuItem icon={<FileVideo size={14} />} label="Insert Video..." shortcut="Ctrl+I" onClick={addVideoTrack} />
              <MenuItem icon={<FileAudio size={14} />} label="Insert Audio..." shortcut="Ctrl+Shift+I" onClick={addAudioTrack} />
              <MenuDivider />
              <MenuItem icon={<Film size={14} />} label="New Video Track" shortcut="Ctrl+Shift+N" onClick={addVideoTrack} />
              <MenuItem icon={<Music size={14} />} label="New Audio Track" shortcut="Ctrl+Shift+A" onClick={addAudioTrack} />
              <MenuItem icon={<Type size={14} />} label="New Subtitle Track" onClick={addSubtitleTrack} />
              <MenuDivider />
              <MenuItem icon={<Download size={14} />} label="Export..." shortcut="Ctrl+E" onClick={() => setIsExporting(true)} />
              {isExporting && <MenuItem icon={<X size={14} />} label="Cancel Export" shortcut="Esc" onClick={() => setIsExporting(false)} />}
            </div>
          )}
        </div>

        {/* Edit Menu */}
        <div className="relative">
          <button
            onClick={(e) => { e.stopPropagation(); setActiveMenu(activeMenu === "edit" ? null : "edit") }}
            className={`px-3 py-1 ${activeMenu === "edit" ? "bg-[#0A84FF] text-white" : "hover:bg-[#3D3D3D] hover:text-white"}`}
          >
            Edit
          </button>
          {activeMenu === "edit" && (
            <div className="absolute left-0 top-full z-50 min-w-[220px] rounded-b-lg bg-[#3D3D3D] py-1 shadow-xl">
              <MenuItem icon={<Undo2 size={14} />} label="Undo" shortcut="Ctrl+Z" />
              <MenuItem icon={<Redo2 size={14} />} label="Redo" shortcut="Ctrl+Shift+Z" />
              <MenuDivider />
              <MenuItem icon={<Scissors size={14} />} label="Split Clip at Playhead" shortcut="Ctrl+B" disabled={!selectedClip} />
              <MenuDivider />
              <MenuItem icon={<ArrowDown size={14} />} label="Move Clip to Track Below" shortcut="Ctrl+Shift+Down" disabled={!selectedClip} />
              <MenuItem icon={<ArrowUp size={14} />} label="Move Clip to Track Above" shortcut="Ctrl+Shift+Up" disabled={!selectedClip} />
              <MenuDivider />
              <MenuItem icon={<RotateCw size={14} />} label="Rotate 90 Right" shortcut="Ctrl+R" disabled={!selectedClip} />
              <MenuItem icon={<RotateCcw size={14} />} label="Rotate 90 Left" shortcut="Ctrl+Shift+R" disabled={!selectedClip} />
              <MenuItem icon={<FlipHorizontal2 size={14} />} label="Flip Horizontal" disabled={!selectedClip} />
              <MenuItem icon={<FlipVertical2 size={14} />} label="Flip Vertical" disabled={!selectedClip} />
              <MenuDivider />
              <MenuItem icon={<Move size={14} />} label="Resize Video..." disabled={!selectedClip} />
              <MenuDivider />
              <div className="group relative">
                <MenuItem icon={<Music size={14} />} label="Audio" hasSubmenu disabled={!selectedClip} />
                <div className="invisible absolute left-full top-0 min-w-[180px] rounded-lg bg-[#3D3D3D] py-1 shadow-xl group-hover:visible">
                  <MenuItem icon={<Trash2 size={14} />} label="Remove Audio" />
                  <MenuItem icon={<FileAudio size={14} />} label="Replace Audio..." />
                  <MenuItem icon={<Plus size={14} />} label="Overlay Audio..." />
                </div>
              </div>
            </div>
          )}
        </div>

        {/* View Menu */}
        <div className="relative">
          <button
            onClick={(e) => { e.stopPropagation(); setActiveMenu(activeMenu === "view" ? null : "view") }}
            className={`px-3 py-1 ${activeMenu === "view" ? "bg-[#0A84FF] text-white" : "hover:bg-[#3D3D3D] hover:text-white"}`}
          >
            View
          </button>
          {activeMenu === "view" && (
            <div className="absolute left-0 top-full z-50 min-w-[220px] rounded-b-lg bg-[#3D3D3D] py-1 shadow-xl">
              <MenuItem 
                icon={<Repeat size={14} />} 
                label="Loop Playback" 
                shortcut="Ctrl+L" 
                checked={isLooping}
                onClick={() => setIsLooping(!isLooping)}
              />
              <MenuDivider />
              <MenuItem icon={<ZoomIn size={14} />} label="Zoom In" shortcut="Ctrl+=" onClick={() => setZoomLevel(Math.min(400, zoomLevel + 25))} />
              <MenuItem icon={<ZoomOut size={14} />} label="Zoom Out" shortcut="Ctrl+-" onClick={() => setZoomLevel(Math.max(25, zoomLevel - 25))} />
              <MenuItem label={`Zoom to Fit (${zoomLevel}%)`} shortcut="Ctrl+0" onClick={() => setZoomLevel(100)} />
              <MenuItem label="Actual Size (1:1)" />
              <MenuDivider />
              <MenuItem 
                icon={isFullscreen ? <Minimize size={14} /> : <Maximize size={14} />} 
                label={isFullscreen ? "Exit Fullscreen" : "Enter Fullscreen"} 
                shortcut="Esc"
                onClick={() => setIsFullscreen(!isFullscreen)}
              />
              <MenuDivider />
              <MenuItem 
                icon={showVideoTracks ? <Eye size={14} /> : <EyeOff size={14} />} 
                label="Video Tracks" 
                checked={showVideoTracks}
                onClick={() => setShowVideoTracks(!showVideoTracks)}
              />
              <MenuItem 
                icon={showAudioTracks ? <Eye size={14} /> : <EyeOff size={14} />} 
                label="Audio Tracks" 
                checked={showAudioTracks}
                onClick={() => setShowAudioTracks(!showAudioTracks)}
              />
              <MenuItem 
                icon={showSubtitleTracks ? <Eye size={14} /> : <EyeOff size={14} />} 
                label="Subtitle Tracks" 
                checked={showSubtitleTracks}
                onClick={() => setShowSubtitleTracks(!showSubtitleTracks)}
              />
              <MenuDivider />
              <MenuItem 
                icon={showStatus ? <Eye size={14} /> : <EyeOff size={14} />} 
                label="Status Bar" 
                checked={showStatus}
                onClick={() => setShowStatus(!showStatus)}
              />
            </div>
          )}
        </div>

        {/* Effects Menu */}
        <div className="relative">
          <button
            onClick={(e) => { e.stopPropagation(); setActiveMenu(activeMenu === "effects" ? null : "effects") }}
            className={`px-3 py-1 ${activeMenu === "effects" ? "bg-[#0A84FF] text-white" : "hover:bg-[#3D3D3D] hover:text-white"}`}
          >
            Effects
          </button>
          {activeMenu === "effects" && (
            <div className="absolute left-0 top-full z-50 min-w-[220px] rounded-b-lg bg-[#3D3D3D] py-1 shadow-xl">
              <MenuItem icon={<User size={14} />} label="Face Swap (FaceFusion)" />
              <MenuItem icon={<Wand2 size={14} />} label="Face Enhance" />
              <MenuItem icon={<Eraser size={14} />} label="Remove Background (RVM)" />
              <MenuDivider />
              <MenuItem icon={<Sparkles size={14} />} label="AI Upscale..." disabled />
            </div>
          )}
        </div>

        {/* Window Menu */}
        <div className="relative">
          <button
            onClick={(e) => { e.stopPropagation(); setActiveMenu(activeMenu === "window" ? null : "window") }}
            className={`px-3 py-1 ${activeMenu === "window" ? "bg-[#0A84FF] text-white" : "hover:bg-[#3D3D3D] hover:text-white"}`}
          >
            Window
          </button>
          {activeMenu === "window" && (
            <div className="absolute left-0 top-full z-50 min-w-[180px] rounded-b-lg bg-[#3D3D3D] py-1 shadow-xl">
              <MenuItem 
                icon={<PanelTop size={14} />} 
                label="Always on Top" 
                checked={alwaysOnTop}
                onClick={() => setAlwaysOnTop(!alwaysOnTop)}
              />
              <MenuDivider />
              <MenuItem label="Fit" />
              <MenuItem label="Fill" />
              <MenuItem label="Center" />
            </div>
          )}
        </div>

        {/* Help Menu */}
        <div className="relative">
          <button
            onClick={(e) => { e.stopPropagation(); setActiveMenu(activeMenu === "help" ? null : "help") }}
            className={`px-3 py-1 ${activeMenu === "help" ? "bg-[#0A84FF] text-white" : "hover:bg-[#3D3D3D] hover:text-white"}`}
          >
            Help
          </button>
          {activeMenu === "help" && (
            <div className="absolute left-0 top-full z-50 min-w-[220px] rounded-b-lg bg-[#3D3D3D] py-1 shadow-xl">
              <MenuItem icon={<HelpCircle size={14} />} label="Overview" shortcut="F1" />
              <MenuItem label="Features" />
              <MenuItem label="Keyboard Shortcuts" />
              <MenuDivider />
              <MenuItem label="Media Formats & Tracks" />
              <MenuItem label="Supported Formats" />
              <MenuDivider />
              <MenuItem label="CLI Reference" />
              <MenuItem label="External AI & Tools" />
              <MenuDivider />
              <MenuItem label="Developers" />
              <MenuItem label="Agents" />
              <MenuItem label="UI Phases" />
            </div>
          )}
        </div>
      </div>

      {/* Video Preview Area - Full height to bottom */}
      <div
        ref={videoContainerRef}
        className="relative flex flex-1 items-center justify-center"
        style={{
          backgroundImage: "url('https://wallpapercave.com/wp/wp4752797.png')",
          backgroundSize: "cover",
          backgroundPosition: "center",
        }}
        onMouseMove={showControls}
        onMouseEnter={showControls}
        onKeyDown={showControls}
      >
        {/* Actual Video Element - hidden until playing */}
        <video
          ref={videoRef}
          src={videoSrc}
          className={`h-full w-full object-contain ${isPlaying ? "opacity-100" : "opacity-0"}`}
          loop={isLooping}
          muted={isMuted}
          onTimeUpdate={(e) => {
            const video = e.currentTarget
            setCurrentTime(video.currentTime * 1000)
          }}
          onLoadedMetadata={(e) => {
            const video = e.currentTarget
            if (video.duration && isFinite(video.duration)) {
              // Update duration based on loaded video
            }
          }}
          onPlay={() => setIsPlaying(true)}
          onPause={() => setIsPlaying(false)}
          onClick={() => {
            if (videoRef.current) {
              if (isPlaying) {
                videoRef.current.pause()
              } else {
                videoRef.current.play()
              }
            }
          }}
          crossOrigin="anonymous"
        />

        {/* QuickTime-style Floating Draggable Controls */}
        <div
          className={`absolute bottom-[10%] left-1/2 flex w-[60%] max-w-2xl flex-col items-center gap-2 rounded-2xl bg-black/50 px-6 py-4 backdrop-blur-md transition-opacity duration-300 ${
            controlsVisible ? "opacity-100" : "pointer-events-none opacity-0"
          }`}
          style={{
            transform: `translate(calc(-50% + ${controlsPosition.x}px), ${controlsPosition.y}px)`,
            cursor: isDragging ? "grabbing" : "default",
          }}
          onMouseDown={(e) => {
            // Only allow dragging from the drag handle
            if ((e.target as HTMLElement).closest("[data-drag-handle]")) {
              handleDragStart(e)
            }
          }}
        >
          {/* Row 1: Transport + Secondary Controls */}
          <div className="flex w-full items-center justify-between">
            {/* Left: Drag Handle + Skip Back */}
            <div className="flex items-center gap-2">
              <div
                data-drag-handle
                className="cursor-grab rounded p-1 text-white/40 transition-colors hover:bg-white/10 hover:text-white/60 active:cursor-grabbing"
                title="Drag to reposition"
              >
                <GripHorizontal size={14} />
              </div>
              <button 
                onClick={() => {
                  setCurrentTime(0)
                  if (videoRef.current) videoRef.current.currentTime = 0
                }}
                className="text-white/70 transition-colors hover:text-white"
              >
                <SkipBack size={18} />
              </button>
            </div>

            {/* Center: Playback Direction + Play/Pause */}
            <div className="flex items-center gap-3">
              <button
                onClick={() => {
                  setIsPlayingBackward(true)
                }}
                className={`rounded-full p-1.5 transition-colors ${
                  isPlayingBackward ? "bg-[#0A84FF]/30 text-[#0A84FF]" : "text-white/70 hover:bg-white/10 hover:text-white"
                }`}
                title="Play Backward"
              >
                <ArrowLeft size={16} />
              </button>

              <button
                onClick={() => {
                  setIsPlayingBackward(false)
                  if (videoRef.current) {
                    if (isPlaying) {
                      videoRef.current.pause()
                    } else {
                      videoRef.current.play()
                    }
                  }
                }}
                className="flex h-10 w-10 items-center justify-center rounded-full bg-white/20 text-white transition-all hover:scale-105 hover:bg-white/30"
              >
                {isPlaying ? <Pause size={20} /> : <Play size={20} className="ml-0.5" />}
              </button>

              <button
                onClick={() => {
                  setIsPlayingBackward(false)
                  if (videoRef.current && !isPlaying) {
                    videoRef.current.play()
                  }
                }}
                className={`rounded-full p-1.5 transition-colors ${
                  !isPlayingBackward && isPlaying ? "bg-[#0A84FF]/30 text-[#0A84FF]" : "text-white/70 hover:bg-white/10 hover:text-white"
                }`}
                title="Play Forward"
              >
                <ArrowRight size={16} />
              </button>
            </div>

            {/* Right: Skip Forward + Speed + Loop + Volume + Fullscreen */}
            <div className="flex items-center gap-2">
              <button 
                onClick={() => {
                  if (videoRef.current) {
                    videoRef.current.currentTime = videoRef.current.duration
                  }
                }}
                className="text-white/70 transition-colors hover:text-white"
              >
                <SkipForward size={18} />
              </button>

              <div className="mx-1 h-4 w-px bg-white/20" />

              {/* Speed selector */}
              <div className="relative">
                <button
                  onClick={() => setShowSpeedMenu(!showSpeedMenu)}
                  className="flex items-center gap-0.5 rounded px-1.5 py-1 text-[11px] text-white/70 transition-colors hover:bg-white/10 hover:text-white"
                >
                  {playbackSpeed}
                  <ChevronDown size={10} />
                </button>
                {showSpeedMenu && (
                  <div className="absolute bottom-full left-1/2 mb-2 -translate-x-1/2 rounded-lg bg-[#3D3D3D] py-1 shadow-xl">
                    {speedOptions.map((speed) => (
                      <button
                        key={speed}
                        onClick={() => {
                          setPlaybackSpeed(speed)
                          setShowSpeedMenu(false)
                          if (videoRef.current) {
                            videoRef.current.playbackRate = parseFloat(speed.replace("×", ""))
                          }
                        }}
                        className={`block w-full px-4 py-1.5 text-left text-xs transition-colors hover:bg-[#4A4A4A] ${
                          playbackSpeed === speed ? "text-[#0A84FF]" : "text-white"
                        }`}
                      >
                        {speed}
                      </button>
                    ))}
                  </div>
                )}
              </div>

              <button
                onClick={() => setIsLooping(!isLooping)}
                className={`rounded p-1 transition-colors ${
                  isLooping ? "bg-[#0A84FF]/30 text-[#0A84FF]" : "text-white/70 hover:bg-white/10 hover:text-white"
                }`}
                title="Loop Playback"
              >
                <Repeat size={14} />
              </button>

              <div className="flex items-center gap-1">
                <button
                  onClick={() => {
                    setIsMuted(!isMuted)
                    if (videoRef.current) videoRef.current.muted = !isMuted
                  }}
                  className="text-white/70 transition-colors hover:text-white"
                >
                  {isMuted ? <VolumeX size={14} /> : <Volume2 size={14} />}
                </button>
                <input
                  type="range"
                  min="0"
                  max="1"
                  step="0.01"
                  value={isMuted ? 0 : volume}
                  onChange={(e) => {
                    const val = parseFloat(e.target.value)
                    setVolume(val)
                    setIsMuted(false)
                    if (videoRef.current) {
                      videoRef.current.volume = val
                      videoRef.current.muted = false
                    }
                  }}
                  className="h-1 w-12 cursor-pointer appearance-none rounded-full bg-white/30 accent-white"
                />
              </div>

              <button
                onClick={() => setIsFullscreen(!isFullscreen)}
                className="text-white/70 transition-colors hover:text-white"
              >
                {isFullscreen ? <Minimize size={14} /> : <Maximize size={14} />}
              </button>

              <div className="mx-1 h-4 w-px bg-white/20" />

              {/* Tools Panel Toggle */}
              <div className="relative">
                <button
                  onClick={() => {
                    setShowToolsPanel(!showToolsPanel)
                    setShowEffectsSubmenu(false)
                  }}
                  className={`rounded p-1 transition-colors ${
                    showToolsPanel ? "bg-[#0A84FF]/30 text-[#0A84FF]" : "text-white/70 hover:bg-white/10 hover:text-white"
                  }`}
                  title="Edit Tools"
                >
                  <ChevronsRight size={14} />
                </button>

                {/* Tools Popup Panel */}
                {showToolsPanel && (
                  <div className="absolute bottom-full right-0 mb-2 min-w-[160px] rounded-lg bg-[#2D2D2D] py-2 shadow-xl">
                    <button className="flex w-full items-center gap-3 px-3 py-2 text-left text-xs text-white transition-colors hover:bg-[#0A84FF]">
                      <Wand2 size={14} />
                      <span>Magic Wand</span>
                    </button>
                    <button className="flex w-full items-center gap-3 px-3 py-2 text-left text-xs text-white transition-colors hover:bg-[#0A84FF]">
                      <Lasso size={14} />
                      <span>Lasso Select</span>
                    </button>
                    <button className="flex w-full items-center gap-3 px-3 py-2 text-left text-xs text-white transition-colors hover:bg-[#0A84FF]">
                      <Crop size={14} />
                      <span>Crop</span>
                    </button>
                    
                    <div className="my-1 h-px bg-[#4A4A4A]" />
                    
                    <button className="flex w-full items-center gap-3 px-3 py-2 text-left text-xs text-white transition-colors hover:bg-[#0A84FF]">
                      <Contrast size={14} />
                      <span>Contrast</span>
                    </button>
                    <button className="flex w-full items-center gap-3 px-3 py-2 text-left text-xs text-white transition-colors hover:bg-[#0A84FF]">
                      <Sun size={14} />
                      <span>Lighting</span>
                    </button>
                    
                    <div className="my-1 h-px bg-[#4A4A4A]" />
                    
                    {/* Effects Submenu */}
                    <div className="relative">
                      <button 
                        className="flex w-full items-center justify-between gap-3 px-3 py-2 text-left text-xs text-white transition-colors hover:bg-[#0A84FF]"
                        onMouseEnter={() => setShowEffectsSubmenu(true)}
                        onMouseLeave={() => setShowEffectsSubmenu(false)}
                      >
                        <div className="flex items-center gap-3">
                          <Layers size={14} />
                          <span>Effects</span>
                        </div>
                        <ChevronRight size={12} />
                      </button>
                      
                      {/* Effects Submenu Popup */}
                      {showEffectsSubmenu && (
                        <div 
                          className="absolute bottom-0 right-full mr-1 min-w-[140px] rounded-lg bg-[#2D2D2D] py-2 shadow-xl"
                          onMouseEnter={() => setShowEffectsSubmenu(true)}
                          onMouseLeave={() => setShowEffectsSubmenu(false)}
                        >
                          <button className="flex w-full items-center gap-3 px-3 py-2 text-left text-xs text-white transition-colors hover:bg-[#0A84FF]">
                            <User size={14} />
                            <span>Face Swap</span>
                          </button>
                          <button className="flex w-full items-center gap-3 px-3 py-2 text-left text-xs text-white transition-colors hover:bg-[#0A84FF]">
                            <Wand2 size={14} />
                            <span>Face Enhance</span>
                          </button>
                          <button className="flex w-full items-center gap-3 px-3 py-2 text-left text-xs text-white transition-colors hover:bg-[#0A84FF]">
                            <Eraser size={14} />
                            <span>Remove BG</span>
                          </button>
                          <div className="my-1 h-px bg-[#4A4A4A]" />
                          <button className="flex w-full items-center gap-3 px-3 py-2 text-left text-xs text-[#6E6E6E]" disabled>
                            <Sparkles size={14} />
                            <span>AI Upscale</span>
                          </button>
                        </div>
                      )}
                    </div>
                  </div>
                )}
              </div>
            </div>
          </div>

          {/* Row 2: Scrub Bar with Time on sides */}
          <div className="flex w-full items-center gap-3">
            <span className="w-14 text-right font-mono text-[11px] text-white">{formatTime(currentTime)}</span>
            <div
              className="relative h-1.5 flex-1 cursor-pointer rounded-full bg-white/20"
              onClick={(e) => {
                const rect = e.currentTarget.getBoundingClientRect()
                const percentage = (e.clientX - rect.left) / rect.width
                if (videoRef.current) {
                  videoRef.current.currentTime = percentage * videoRef.current.duration
                }
              }}
            >
              <div
                className="absolute h-full rounded-full bg-white"
                style={{ width: `${playheadPosition}%` }}
              />
              <div
                className="absolute top-1/2 h-3 w-3 -translate-x-1/2 -translate-y-1/2 rounded-full bg-white shadow-lg"
                style={{ left: `${playheadPosition}%` }}
              />
            </div>
            <span className="w-14 font-mono text-[11px] text-white/60">{formatTime(totalDuration)}</span>
          </div>
        </div>
      </div>

      {/* Timeline - only show if at least one track type is visible */}
      {(showVideoTracks || showAudioTracks || showSubtitleTracks) && (
      <div className="min-h-[120px] overflow-auto bg-[#1E1E1E] px-4 py-3">
        {/* Video Tracks Section */}
        {showVideoTracks && (
        <div className="mb-3">
          <div className="mb-1 flex items-center justify-between">
            <span className="text-[10px] font-medium uppercase tracking-wider text-[#6E6E6E]">Video</span>
            <button
              onClick={addVideoTrack}
              className="flex h-5 w-5 items-center justify-center rounded bg-[#3D3D3D] text-[#A0A0A0] transition-colors hover:bg-[#4A4A4A] hover:text-white"
              title="Add Video Track"
            >
              <Plus size={12} />
            </button>
          </div>
          {videoTracks.map((track) => (
            <div 
              key={track.id} 
              className="group/track mb-1 flex overflow-hidden rounded bg-[#252525]"
              onMouseEnter={() => setHoveredTrack(`video-${track.id}`)}
              onMouseLeave={() => setHoveredTrack(null)}
            >
              {/* Track label */}
              <div className="relative flex w-24 flex-shrink-0 items-center gap-2 bg-[#2D2D2D] px-3 py-2">
                <Film size={14} className="text-[#4A90D9]" />
                <span className="truncate text-xs text-[#A0A0A0]">{track.name}</span>
                {/* Delete button on hover */}
                <button
                  onClick={(e) => { e.stopPropagation(); deleteVideoTrack(track.id) }}
                  className={`absolute right-1 top-1/2 -translate-y-1/2 rounded p-1 text-[#FF453A] transition-opacity hover:bg-[#FF453A]/20 ${
                    hoveredTrack === `video-${track.id}` ? "opacity-100" : "opacity-0"
                  }`}
                  title="Delete Track"
                >
                  <Trash2 size={12} />
                </button>
              </div>

              {/* Clips area */}
              <div className="relative flex flex-1 gap-0.5 overflow-hidden p-1">
                {track.clips.length === 0 ? (
                  <div className="flex h-12 w-full items-center justify-center text-[10px] text-[#4A4A4A]">
                    Drop clips here
                  </div>
                ) : (
                  track.clips.map((clip) => (
                    <div
                      key={clip.id}
                      onClick={() => setSelectedClip(selectedClip === clip.id ? null : clip.id)}
                      className={`relative h-12 cursor-pointer overflow-hidden rounded transition-all ${
                        selectedClip === clip.id ? "ring-2 ring-[#FFD60A]" : ""
                      }`}
                      style={{
                        width: `${(clip.duration / totalDuration) * 100}%`,
                        background: `linear-gradient(180deg, ${clip.color} 0%, ${clip.color}dd 100%)`,
                      }}
                    >
                      {/* Filmstrip effect */}
                      <div className="absolute inset-0 flex">
                        {Array.from({ length: Math.ceil(clip.duration / 2000) }).map((_, i) => (
                          <div
                            key={i}
                            className="h-full flex-1 border-r border-black/20"
                            style={{ opacity: 0.7 + Math.random() * 0.3 }}
                          />
                        ))}
                      </div>

                      {/* Clip name */}
                      <div className="absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/60 to-transparent px-2 py-1">
                        <span className="truncate text-[9px] text-white">{clip.name}</span>
                      </div>

                      {/* Trim handles (visible when selected) */}
                      {selectedClip === clip.id && (
                        <>
                          <div className="absolute left-0 top-0 h-full w-1.5 cursor-ew-resize bg-[#FFD60A]">
                            <GripVertical size={10} className="absolute left-0 top-1/2 -translate-y-1/2 text-black/50" />
                          </div>
                          <div className="absolute right-0 top-0 h-full w-1.5 cursor-ew-resize bg-[#FFD60A]">
                            <GripVertical size={10} className="absolute right-0 top-1/2 -translate-y-1/2 text-black/50" />
                          </div>
                        </>
                      )}
                    </div>
                  ))
                )}

                {/* Playhead */}
                <div
                  className="pointer-events-none absolute top-0 h-full w-0.5 bg-white shadow-[0_0_8px_rgba(255,255,255,0.5)]"
                  style={{ left: `${playheadPosition}%` }}
                >
                  {track.id === videoTracks[0].id && (
                    <div className="absolute -left-1.5 -top-1 h-2 w-3 bg-white" style={{ clipPath: "polygon(50% 100%, 0 0, 100% 0)" }} />
                  )}
                </div>
              </div>
            </div>
          ))}
        </div>
        )}

        {/* Audio Tracks Section */}
        {showAudioTracks && (
        <div className="mb-3">
          <div className="mb-1 flex items-center justify-between">
            <span className="text-[10px] font-medium uppercase tracking-wider text-[#6E6E6E]">Audio</span>
            <button
              onClick={addAudioTrack}
              className="flex h-5 w-5 items-center justify-center rounded bg-[#3D3D3D] text-[#A0A0A0] transition-colors hover:bg-[#4A4A4A] hover:text-white"
              title="Add Audio Track"
            >
              <Plus size={12} />
            </button>
          </div>
          {audioTracks.map((track) => (
            <div 
              key={track.id} 
              className="group/track mb-1 flex overflow-hidden rounded bg-[#252525]"
              onMouseEnter={() => setHoveredTrack(`audio-${track.id}`)}
              onMouseLeave={() => setHoveredTrack(null)}
            >
              {/* Track label */}
              <div className="relative flex w-24 flex-shrink-0 items-center gap-2 bg-[#2D2D2D] px-3 py-2">
                <Music size={14} className="text-[#5AC8FA]" />
                <span className="truncate text-xs text-[#A0A0A0]">{track.name}</span>
                {/* Delete button on hover */}
                <button
                  onClick={(e) => { e.stopPropagation(); deleteAudioTrack(track.id) }}
                  className={`absolute right-1 top-1/2 -translate-y-1/2 rounded p-1 text-[#FF453A] transition-opacity hover:bg-[#FF453A]/20 ${
                    hoveredTrack === `audio-${track.id}` ? "opacity-100" : "opacity-0"
                  }`}
                  title="Delete Track"
                >
                  <Trash2 size={12} />
                </button>
              </div>

              {/* Clips area */}
              <div className="relative flex flex-1 gap-0.5 overflow-hidden p-1">
                {track.clips.length === 0 ? (
                  <div className="flex h-12 w-full items-center justify-center text-[10px] text-[#4A4A4A]">
                    Drop clips here
                  </div>
                ) : (
                  track.clips.map((clip) => (
                    <div
                      key={clip.id}
                      className="relative h-12 overflow-hidden rounded"
                      style={{
                        width: `${(clip.duration / totalDuration) * 100}%`,
                        background: `linear-gradient(180deg, ${clip.color} 0%, ${clip.color}dd 100%)`,
                      }}
                    >
                      {/* Waveform simulation */}
                      <div className="absolute inset-0 flex items-center justify-center">
                        <svg className="h-full w-full" preserveAspectRatio="none">
                          <path
                            d={`M 0 24 ${Array.from({ length: 50 })
                              .map((_, i) => {
                                const x = (i / 50) * 100
                                const y = 24 + Math.sin(i * 0.5) * 8 + Math.random() * 8
                                return `L ${x} ${y}`
                              })
                              .join(" ")}`}
                            stroke="rgba(255,255,255,0.6)"
                            strokeWidth="1"
                            fill="none"
                          />
                        </svg>
                      </div>

                      {/* Clip name */}
                      <div className="absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/60 to-transparent px-2 py-1">
                        <span className="truncate text-[9px] text-white">{clip.name}</span>
                      </div>
                    </div>
                  ))
                )}

                {/* Playhead */}
                <div
                  className="pointer-events-none absolute top-0 h-full w-0.5 bg-white shadow-[0_0_8px_rgba(255,255,255,0.5)]"
                  style={{ left: `${playheadPosition}%` }}
                />
              </div>
            </div>
          ))}
        </div>
        )}

        {/* Subtitle Tracks Section */}
        {showSubtitleTracks && (
        <div>
          <div className="mb-1 flex items-center justify-between">
            <span className="text-[10px] font-medium uppercase tracking-wider text-[#6E6E6E]">Subtitles</span>
            <button
              onClick={addSubtitleTrack}
              className="flex h-5 w-5 items-center justify-center rounded bg-[#3D3D3D] text-[#A0A0A0] transition-colors hover:bg-[#4A4A4A] hover:text-white"
              title="Add Subtitle Track"
            >
              <Plus size={12} />
            </button>
          </div>
          {subtitleTracks.map((track) => (
            <div 
              key={track.id} 
              className="group/track mb-1 flex overflow-hidden rounded bg-[#252525]"
              onMouseEnter={() => setHoveredTrack(`subtitle-${track.id}`)}
              onMouseLeave={() => setHoveredTrack(null)}
            >
              {/* Track label */}
              <div className="relative flex w-24 flex-shrink-0 items-center gap-2 bg-[#2D2D2D] px-3 py-2">
                <Type size={14} className="text-[#FF9F0A]" />
                <span className="truncate text-xs text-[#A0A0A0]">{track.name}</span>
                {/* Delete button on hover */}
                <button
                  onClick={(e) => { e.stopPropagation(); deleteSubtitleTrack(track.id) }}
                  className={`absolute right-1 top-1/2 -translate-y-1/2 rounded p-1 text-[#FF453A] transition-opacity hover:bg-[#FF453A]/20 ${
                    hoveredTrack === `subtitle-${track.id}` ? "opacity-100" : "opacity-0"
                  }`}
                  title="Delete Track"
                >
                  <Trash2 size={12} />
                </button>
              </div>

              {/* Clips area */}
              <div className="relative flex flex-1 gap-0.5 overflow-hidden p-1">
                {track.clips.length === 0 ? (
                  <div className="flex h-8 w-full items-center justify-center text-[10px] text-[#4A4A4A]">
                    Drop subtitle files here
                  </div>
                ) : (
                  track.clips.map((clip) => (
                    <div
                      key={clip.id}
                      className="relative h-8 overflow-hidden rounded border border-[#FF9F0A]/30"
                      style={{
                        width: `${(clip.duration / totalDuration) * 100}%`,
                        marginLeft: clip.offset ? `${(clip.offset / totalDuration) * 100}%` : 0,
                        background: `linear-gradient(180deg, ${clip.color}40 0%, ${clip.color}20 100%)`,
                      }}
                    >
                      {/* Subtitle markers */}
                      <div className="absolute inset-0 flex items-center px-2">
                        <div className="flex h-2 w-full items-center gap-1">
                          {Array.from({ length: 8 }).map((_, i) => (
                            <div
                              key={i}
                              className="h-1.5 flex-1 rounded-sm bg-[#FF9F0A]/60"
                            />
                          ))}
                        </div>
                      </div>

                      {/* Clip name */}
                      <div className="absolute inset-x-0 bottom-0 px-2 py-0.5">
                        <span className="truncate text-[8px] text-[#FF9F0A]">{clip.name}</span>
                      </div>
                    </div>
                  ))
                )}

                {/* Playhead */}
                <div
                  className="pointer-events-none absolute top-0 h-full w-0.5 bg-white shadow-[0_0_8px_rgba(255,255,255,0.5)]"
                  style={{ left: `${playheadPosition}%` }}
                />
              </div>
            </div>
          ))}
        </div>
        )}
      </div>
      )}

      {/* Action Toolbar (contextual) */}
      {selectedClip && (
        <div className="flex items-center justify-center gap-2 border-t border-[#3A3A3A] bg-[#252525] py-2">
          <button
            onClick={() => setIsTrimMode(true)}
            className="flex items-center gap-1.5 rounded px-3 py-1.5 text-xs text-[#A0A0A0] transition-colors hover:bg-[#3D3D3D] hover:text-white"
          >
            <Scissors size={14} />
            Split (⌘B)
          </button>
          <div className="h-4 w-px bg-[#3A3A3A]" />
          <button className="flex items-center gap-1.5 rounded px-3 py-1.5 text-xs text-[#A0A0A0] transition-colors hover:bg-[#3D3D3D] hover:text-white">
            <RotateCw size={14} />
            Rotate Right (⌘R)
          </button>
          <button className="flex items-center gap-1.5 rounded px-3 py-1.5 text-xs text-[#A0A0A0] transition-colors hover:bg-[#3D3D3D] hover:text-white">
            <RotateCcw size={14} />
            Rotate Left
          </button>
          <div className="h-4 w-px bg-[#3A3A3A]" />
          <button className="flex items-center gap-1.5 rounded px-3 py-1.5 text-xs text-[#A0A0A0] transition-colors hover:bg-[#3D3D3D] hover:text-white">
            <FlipHorizontal2 size={14} />
            Flip H
          </button>
          <button className="flex items-center gap-1.5 rounded px-3 py-1.5 text-xs text-[#A0A0A0] transition-colors hover:bg-[#3D3D3D] hover:text-white">
            <FlipVertical2 size={14} />
            Flip V
          </button>
        </div>
      )}

      {/* Status Footer */}
      {showStatus && (
      <div className="flex h-6 items-center justify-between border-t border-[#3A3A3A] bg-[#1E1E1E] px-3 text-[11px]">
        {/* Codec info */}
        <div className="flex items-center gap-2 text-[#A0A0A0]">
          <span>H.264</span>
          <div className="h-3 w-px bg-[#3A3A3A]" />
          <span>AAC</span>
        </div>

        {/* Project path */}
        <span className="text-[#6E6E6E]">~/Projects/MyVideo.reel</span>

        {/* Save status */}
        <div className="flex items-center gap-1.5">
          {isSaved ? (
            <>
              <Check size={12} className="text-[#30D158]" />
              <span className="text-[#30D158]">All changes saved</span>
            </>
          ) : (
            <>
              <Circle size={12} className="text-[#6E6E6E]" />
              <span className="text-[#A0A0A0]">Unsaved changes</span>
            </>
          )}
        </div>
      </div>
      )}

    </div>
  )
}
