import { useCallback, useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { open } from '@tauri-apps/plugin-dialog'
import {
  AlertCircle,
  Bell,
  Check,
  ChevronDown,
  Clock3,
  CloudUpload,
  Cpu,
  FileAudio,
  FileText,
  FileImage,
  FileVideo,
  FolderOpen,
  HardDrive,
  Pause,
  Play,
  RotateCcw,
  Search,
  Settings2,
  SlidersHorizontal,
  X,
} from 'lucide-react'
import './App.css'

type FileCategory = 'video' | 'audio' | 'image' | 'document' | 'unsupported'
type JobStatus = 'queued' | 'running' | 'done' | 'failed' | 'canceled'
type QualityMode = 'fastRemux' | 'fastEncode' | 'smallSize' | 'balanced' | 'highQuality' | 'keepSource'
type OverwritePolicy = 'rename' | 'overwrite'
type AppMode = 'media' | 'documents'
type DocumentOperation = 'imagesToPdf' | 'mergePdfs'

type ConversionPreset = {
  name: string
  description: string
  qualityMode: QualityMode
  overwritePolicy: OverwritePolicy
}

type AdvancedOptions = {
  videoCodec?: string | null
  audioCodec?: string | null
  videoQuality?: number | null
  videoBitrate?: string | null
  audioBitrate?: string | null
  maxWidth?: number | null
  imageQuality?: number | null
  copyStreams: boolean
}

type ConversionJob = {
  id: string
  inputPath: string
  inputPaths: string[]
  outputPath: string
  sourceFormat: string
  targetFormat: string
  category: FileCategory
  preset: ConversionPreset
  advancedOptions?: AdvancedOptions | null
  documentOperation?: DocumentOperation | null
  status: JobStatus
  progress: number
  speed?: string | null
  etaSeconds?: number | null
  error?: string | null
}

type SupportedFormats = {
  video: string[]
  audio: string[]
  image: string[]
  document: string[]
  presets: ConversionPreset[]
  defaultParallelism: number
}

type CodecOption = {
  id: string
  label: string
  available: boolean
  hardware: boolean
}

type FormatCodecMatrix = {
  targetFormat: string
  videoCodecs: CodecOption[]
  audioCodecs: CodecOption[]
  supportsVideo: boolean
  supportsAudio: boolean
  supportsRemux: boolean
}

type ConversionCapabilities = {
  ffmpegAvailable: boolean
  ffmpegPath?: string | null
  hardwareAccels: string[]
  videoEncoders: CodecOption[]
  audioEncoders: CodecOption[]
  matrix: FormatCodecMatrix[]
  warnings: string[]
}

type QueueUpdate = {
  job: ConversionJob
}

type BatchGroup = {
  category: MediaCategory
  paths: string[]
  targetFormat: string
  presetName: string
  advancedOpen: boolean
  advancedOptions: AdvancedOptions | null
}

type MediaCategory = Exclude<FileCategory, 'document' | 'unsupported'>

type DocumentSetup = {
  paths: string[]
  operation: DocumentOperation
  outputDir: string
  outputName: string
}

type CreateJobGroup = {
  paths: string[]
  options: {
    outputDir: string | null
    targetFormat: string
    preset: ConversionPreset
    advancedOptions: AdvancedOptions | null
    parallelism: number
    ffmpegPath: string | null
  }
}

const fallbackPreset: ConversionPreset = {
  name: 'Balanced',
  description: 'Good default quality, size, and compatibility for everyday conversion.',
  qualityMode: 'balanced',
  overwritePolicy: 'rename',
}

const fallbackFormats: SupportedFormats = {
  video: ['mp4', 'mkv', 'mov', 'webm', 'avi'],
  audio: ['mp3', 'aac', 'm4a', 'ogg', 'opus', 'wav', 'flac'],
  image: ['png', 'jpg', 'jpeg', 'webp', 'bmp', 'tiff'],
  document: ['pdf'],
  presets: [
    {
      name: 'Fast remux',
      description: 'Changes the container without re-encoding when streams are compatible.',
      qualityMode: 'fastRemux',
      overwritePolicy: 'rename',
    },
    {
      name: 'Fast encode',
      description: 'Prioritizes speed with moderate compression and broadly compatible codecs.',
      qualityMode: 'fastEncode',
      overwritePolicy: 'rename',
    },
    fallbackPreset,
    {
      name: 'Small size',
      description: 'Smaller files with more compression and lower audio bitrates.',
      qualityMode: 'smallSize',
      overwritePolicy: 'rename',
    },
    {
      name: 'High quality',
      description: 'Preserves more detail with larger files and slower encoding.',
      qualityMode: 'highQuality',
      overwritePolicy: 'rename',
    },
    {
      name: 'Keep source quality',
      description: 'Copies streams when possible, otherwise uses conservative quality settings.',
      qualityMode: 'keepSource',
      overwritePolicy: 'rename',
    },
  ],
  defaultParallelism: 2,
}

const fallbackCapabilities: ConversionCapabilities = {
  ffmpegAvailable: false,
  ffmpegPath: null,
  hardwareAccels: [],
  videoEncoders: [],
  audioEncoders: [],
  matrix: [],
  warnings: ['FFmpeg capability detection has not run yet.'],
}

const defaultTargets: Record<MediaCategory, string> = {
  video: 'webm',
  audio: 'flac',
  image: 'webp',
}

const audioBitrates = ['96k', '128k', '160k', '192k', '256k', '320k']
const maxWidths = ['Original', '720', '1280', '1920', '3840']
const qualityLevels = ['20', '40', '60', '80', '95']

function App() {
  const [activeMode, setActiveMode] = useState<AppMode>('media')
  const [formats, setFormats] = useState<SupportedFormats>(fallbackFormats)
  const [capabilities, setCapabilities] = useState<ConversionCapabilities>(fallbackCapabilities)
  const [jobs, setJobs] = useState<ConversionJob[]>([])
  const [targetFormat, setTargetFormat] = useState('mp4')
  const [presetName, setPresetName] = useState('Balanced')
  const [outputDir, setOutputDir] = useState('')
  const [parallelism, setParallelism] = useState(2)
  const [ffmpegPath, setFfmpegPath] = useState('')
  const [hardwareAcceleration, setHardwareAcceleration] = useState(true)
  const [isRunning, setIsRunning] = useState(false)
  const [dragActive, setDragActive] = useState(false)
  const [lastError, setLastError] = useState<string | null>(null)
  const [isSettingsOpen, setIsSettingsOpen] = useState(false)
  const [batchGroups, setBatchGroups] = useState<BatchGroup[]>([])
  const [activeGroupIndex, setActiveGroupIndex] = useState(0)
  const [completedGroupIndexes, setCompletedGroupIndexes] = useState<number[]>([])
  const [batchOutputDir, setBatchOutputDir] = useState('')
  const [documentSetup, setDocumentSetup] = useState<DocumentSetup | null>(null)

  const selectedPreset = useMemo(
    () => formats.presets.find((preset) => preset.name === presetName) ?? formats.presets[0] ?? fallbackPreset,
    [formats.presets, presetName],
  )

  const summary = useMemo(() => {
    const completed = jobs.filter((job) => job.status === 'done').length
    const failed = jobs.filter((job) => job.status === 'failed').length
    const running = jobs.filter((job) => job.status === 'running').length
    const average = jobs.length ? jobs.reduce((sum, job) => sum + job.progress, 0) / jobs.length : 0
    return { completed, failed, running, average }
  }, [jobs])

  const activeGroup = batchGroups[activeGroupIndex]
  const isBatchModalOpen = batchGroups.length > 0
  const isDocumentModalOpen = documentSetup !== null

  const queueOptions = useCallback(() => ({
    outputDir: outputDir || null,
    targetFormat,
    preset: selectedPreset,
    advancedOptions: null,
    parallelism,
    ffmpegPath: ffmpegPath || null,
  }), [ffmpegPath, outputDir, parallelism, selectedPreset, targetFormat])

  useEffect(() => {
    invoke<SupportedFormats>('get_supported_formats')
      .then((value) => {
        setFormats(value)
        setParallelism(value.defaultParallelism)
      })
      .catch(() => setFormats(fallbackFormats))

    const unlistenPromise = listen<QueueUpdate>('queue://job-updated', (event) => {
      setJobs((current) =>
        current.map((job) => (job.id === event.payload.job.id ? event.payload.job : job)),
      )
    })

    return () => {
      unlistenPromise.then((unlisten) => unlisten()).catch(() => undefined)
    }
  }, [])

  useEffect(() => {
    invoke<ConversionCapabilities>('get_conversion_capabilities', { ffmpegPath: ffmpegPath || null })
      .then(setCapabilities)
      .catch((error) => setCapabilities({ ...fallbackCapabilities, warnings: [String(error)] }))
  }, [ffmpegPath])

  const categoryForPath = useCallback((path: string): FileCategory => {
    const ext = path.split('.').pop()?.toLowerCase() ?? ''
    if (formats.video.includes(ext)) return 'video'
    if (formats.audio.includes(ext)) return 'audio'
    if (formats.image.includes(ext)) return 'image'
    return 'unsupported'
  }, [formats.audio, formats.image, formats.video])

  const preferredTarget = useCallback((category: MediaCategory) => {
    const options = formats[category]
    return options.includes(defaultTargets[category]) ? defaultTargets[category] : options[0]
  }, [formats])

  const presetByName = useCallback((name: string) => {
    return formats.presets.find((preset) => preset.name === name) ?? formats.presets[0] ?? fallbackPreset
  }, [formats.presets])

  const buildBatchGroups = useCallback((paths: string[]): BatchGroup[] => {
    const buckets: Record<MediaCategory, string[]> = {
      video: [],
      audio: [],
      image: [],
    }

    for (const path of paths) {
      const category = categoryForPath(path)
      if (category !== 'unsupported' && category !== 'document') {
        buckets[category].push(path)
      }
    }

    return (['video', 'audio', 'image'] as const)
      .filter((category) => buckets[category].length > 0)
      .map((category) => ({
        category,
        paths: buckets[category],
        targetFormat: preferredTarget(category),
        presetName,
        advancedOpen: false,
        advancedOptions: null,
      }))
  }, [categoryForPath, preferredTarget, presetName])

  const addMediaPaths = useCallback(async (paths: string[]) => {
    if (!paths.length) return
    setLastError(null)

    const grouped = buildBatchGroups(paths)
    const unsupportedCount = paths.length - grouped.reduce((count, group) => count + group.paths.length, 0)
    if (unsupportedCount > 0) {
      setLastError(`${unsupportedCount} unsupported file${unsupportedCount === 1 ? '' : 's'} skipped.`)
    }
    if (!grouped.length) return

    setBatchGroups(grouped)
    setActiveGroupIndex(0)
    setCompletedGroupIndexes([])
    setBatchOutputDir(outputDir)
  }, [buildBatchGroups, outputDir])

  const addDocumentPaths = useCallback(async (paths: string[]) => {
    if (!paths.length) return
    setLastError(null)

    const supported = paths.filter((path) => isDocumentInput(path, formats))
    const unsupportedCount = paths.length - supported.length
    if (unsupportedCount > 0) {
      setLastError(`${unsupportedCount} unsupported document input${unsupportedCount === 1 ? '' : 's'} skipped.`)
    }
    if (!supported.length) return

    const hasImages = supported.some((path) => isImagePath(path, formats))
    const operation: DocumentOperation = hasImages ? 'imagesToPdf' : 'mergePdfs'
    setDocumentSetup({
      paths: supported,
      operation,
      outputDir,
      outputName: operation === 'imagesToPdf' ? 'images.pdf' : 'merged.pdf',
    })
  }, [formats, outputDir])

  const addPaths = useCallback(async (paths: string[]) => {
    if (activeMode === 'documents') {
      await addDocumentPaths(paths)
    } else {
      await addMediaPaths(paths)
    }
  }, [activeMode, addDocumentPaths, addMediaPaths])

  useEffect(() => {
    const unlistenPromise = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === 'enter' || event.payload.type === 'over') {
        setDragActive(true)
      }
      if (event.payload.type === 'leave') {
        setDragActive(false)
      }
      if (event.payload.type === 'drop') {
        setDragActive(false)
        void addPaths(event.payload.paths)
      }
    })

    return () => {
      unlistenPromise.then((unlisten) => unlisten()).catch(() => undefined)
    }
  }, [addPaths])

  async function chooseFiles() {
    const selected = await open({
      multiple: true,
      filters: [
        {
          name: activeMode === 'documents' ? 'Images and PDFs' : 'Media and images',
          extensions: activeMode === 'documents'
            ? [...formats.image, ...formats.document]
            : [...formats.video, ...formats.audio, ...formats.image],
        },
      ],
    })
    const paths = Array.isArray(selected) ? selected : selected ? [selected] : []
    await addPaths(paths)
  }

  async function chooseOutputDir() {
    const selected = await open({ directory: true, multiple: false })
    if (typeof selected === 'string') {
      setOutputDir(selected)
    }
  }

  async function chooseBatchOutputDir() {
    const selected = await open({ directory: true, multiple: false })
    if (typeof selected === 'string') {
      setBatchOutputDir(selected)
    }
  }

  async function chooseDocumentOutputDir() {
    const selected = await open({ directory: true, multiple: false })
    if (typeof selected === 'string') {
      setDocumentSetup((current) => current ? { ...current, outputDir: selected } : current)
    }
  }

  async function chooseFfmpeg() {
    const selected = await open({ multiple: false })
    if (typeof selected === 'string') {
      setFfmpegPath(selected)
    }
  }

  async function confirmBatchSetup() {
    const groups: CreateJobGroup[] = batchGroups.map((group) => ({
      paths: group.paths,
      options: {
        outputDir: batchOutputDir || null,
        targetFormat: group.targetFormat,
        preset: presetByName(group.presetName),
        advancedOptions: normalizedAdvancedOptions(group),
        parallelism,
        ffmpegPath: ffmpegPath || null,
      },
    }))

    try {
      const created = await invoke<ConversionJob[]>('create_jobs_batch', { groups })
      setJobs(created)
      setOutputDir(batchOutputDir)
      setTargetFormat(batchGroups[batchGroups.length - 1]?.targetFormat ?? targetFormat)
      setPresetName(batchGroups[batchGroups.length - 1]?.presetName ?? presetName)
      closeBatchModal()
    } catch (error) {
      setLastError(String(error))
    }
  }

  async function confirmDocumentSetup() {
    if (!documentSetup) return
    const paths = relevantDocumentPaths(documentSetup, formats)
    if (!paths.length) {
      setLastError(documentSetup.operation === 'imagesToPdf' ? 'Add at least one image.' : 'Add at least one PDF.')
      return
    }

    try {
      const created = await invoke<ConversionJob[]>('create_document_job', {
        options: {
          paths,
          outputDir: documentSetup.outputDir || null,
          outputName: documentSetup.outputName || null,
          operation: documentSetup.operation,
          parallelism,
        },
      })
      setJobs(created)
      setOutputDir(documentSetup.outputDir)
      setDocumentSetup(null)
    } catch (error) {
      setLastError(String(error))
    }
  }

  function closeBatchModal() {
    setBatchGroups([])
    setActiveGroupIndex(0)
    setCompletedGroupIndexes([])
  }

  function updateActiveGroup(update: Partial<Pick<BatchGroup, 'targetFormat' | 'presetName' | 'advancedOpen' | 'advancedOptions'>>) {
    setBatchGroups((current) =>
      current.map((group, index) => index === activeGroupIndex ? { ...group, ...update } : group),
    )
  }

  function continueBatchSetup() {
    setCompletedGroupIndexes((current) => Array.from(new Set([...current, activeGroupIndex])))
    setActiveGroupIndex((index) => index + 1)
  }

  function normalizedAdvancedOptions(group: BatchGroup): AdvancedOptions | null {
    if (!group.advancedOpen || !group.advancedOptions) return null
    const options = group.advancedOptions
    if (group.category === 'image') {
      return {
        copyStreams: false,
        imageQuality: clampQuality(options.imageQuality ?? 86),
      }
    }
    if (group.category === 'audio') {
      return {
        copyStreams: options.copyStreams,
        audioCodec: allowedAudioCodecs(group.targetFormat, capabilities)[0]?.id,
        audioBitrate: losslessAudioOnly(group.targetFormat, capabilities) 
          ? null
          : allowedBitrate(options.audioBitrate),
      }
    }
    return {
      copyStreams: options.copyStreams,
      videoCodec: allowedVideoCodecs(group.targetFormat, capabilities)[0]?.id,
      audioCodec: allowedAudioCodecs(group.targetFormat, capabilities)[0]?.id,
      videoQuality: clampQuality(options.videoQuality ?? 60),
      audioBitrate: losslessAudioOnly(group.targetFormat, capabilities)
        ? null
        : allowedBitrate(options.audioBitrate),
      maxWidth: allowedMaxWidth(options.maxWidth),
    }
  }

  function ensureAdvancedOptions(group: BatchGroup): AdvancedOptions {
    return group.advancedOptions ?? {
      copyStreams: false,
      videoCodec: allowedVideoCodecs(group.targetFormat, capabilities)[0]?.id,
      audioCodec: allowedAudioCodecs(group.targetFormat, capabilities)[0]?.id,
      videoQuality: 60,
      audioBitrate: '192k',
      maxWidth: null,
      imageQuality: 86,
    }
  }

  function updateAdvancedOptions(update: Partial<AdvancedOptions>) {
    if (!activeGroup) return
    const next = { ...ensureAdvancedOptions(activeGroup), ...update }
    updateActiveGroup({ advancedOptions: next })
  }

  function sanitizeAdvancedForFormat(group: BatchGroup, targetFormat: string): AdvancedOptions | null {
    if (!group.advancedOptions) return null
    const current = group.advancedOptions
    return {
      ...current,
      videoCodec: allowedVideoCodecs(targetFormat, capabilities)[0]?.id,
      audioCodec: allowedAudioCodecs(targetFormat, capabilities)[0]?.id,
      audioBitrate: losslessAudioOnly(targetFormat, capabilities) ? null : allowedBitrate(current.audioBitrate),
    }
  }

  async function startQueue() {
    if (!jobs.length) return
    setIsRunning(true)
    setLastError(null)
    try {
      const completed = await invoke<ConversionJob[]>('start_queue', { options: queueOptions() })
      setJobs((current) =>
        current.map((job) => completed.find((item) => item.id === job.id) ?? job),
      )
    } catch (error) {
      setLastError(String(error))
    } finally {
      setIsRunning(false)
    }
  }

  async function pauseQueue() {
    await invoke('pause_queue')
    setIsRunning(false)
  }

  async function cancelJob(id: string) {
    const canceled = await invoke<ConversionJob | null>('cancel_job', { id })
    if (canceled) {
      setJobs((current) => current.map((job) => (job.id === id ? canceled : job)))
    }
  }

  async function openOutput(path: string) {
    await invoke('open_output_folder', { path })
  }

  function clearDone() {
    setJobs((current) => current.filter((job) => !['done', 'failed', 'canceled'].includes(job.status)))
  }

  function resetQueue() {
    setJobs([])
    setLastError(null)
  }

  return (
    <main className="app-shell">
      <header className="top-nav">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true">↔</span>
          <span>SHIFTR</span>
        </div>

        <div className="mode-switch" role="tablist" aria-label="Conversion mode">
          <button
            className={activeMode === 'media' ? 'active' : ''}
            onClick={() => setActiveMode('media')}
            role="tab"
            type="button"
          >
            <FileVideo size={16} /> Media
          </button>
          <button
            className={activeMode === 'documents' ? 'active' : ''}
            onClick={() => setActiveMode('documents')}
            role="tab"
            type="button"
          >
            <FileText size={16} /> Documents
          </button>
        </div>

        <div className="search-box">
          <Search size={18} />
          <input placeholder="Search files or presets..." aria-label="Search files or presets" />
        </div>

        <div className="nav-actions">
          <button className="nav-icon" type="button" onClick={() => setIsSettingsOpen(true)} title="Settings">
            <Settings2 size={22} />
          </button>
          <button className="nav-icon" type="button" title="Notifications">
            <Bell size={21} />
          </button>
        </div>
      </header>

      <section className="workspace">
        <div
          className={dragActive ? 'dropzone active' : 'dropzone'}
          onClick={chooseFiles}
          onDragEnter={(event) => {
            event.preventDefault()
            setDragActive(true)
          }}
          onDragOver={(event) => event.preventDefault()}
          onDragLeave={() => setDragActive(false)}
          onDrop={(event) => {
            event.preventDefault()
            setDragActive(false)
            const paths = Array.from(event.dataTransfer.files)
              .map((file) => 'path' in file ? String((file as File & { path?: string }).path) : '')
              .filter(Boolean)
            void addPaths(paths)
          }}
        >
          <div className="drop-icon">
            <CloudUpload size={24} />
          </div>
          <div className="drop-copy">
            <strong>{activeMode === 'documents' ? 'Drag & Drop images or PDFs here' : 'Drag & Drop media here'}</strong>
            <span>{activeMode === 'documents' ? 'create PDFs or merge existing documents' : 'or click to browse local files'}</span>
          </div>
        </div>

        {lastError && <div className="error-strip">{lastError}</div>}

        <section className="queue-header">
          <h2>Batch Queue</h2>
          <span>{jobs.length} Item{jobs.length === 1 ? '' : 's'}</span>
        </section>

        <section className="queue-toolbar">
          <button type="button" onClick={clearDone} disabled={!jobs.length}>
            <RotateCcw size={16} /> Clear finished
          </button>
          <button type="button" onClick={pauseQueue} disabled={!isRunning}>
            <Pause size={16} /> Pause
          </button>
        </section>

        <section className="queue-list">
          {jobs.length === 0 ? (
            <div className="empty-state">
              <CloudUpload size={26} />
              <span>Select files to build a conversion queue.</span>
            </div>
          ) : (
            jobs.map((job) => (
              <article className={`job-row ${job.status}`} key={job.id}>
                <div className="job-icon">{iconFor(job.category)}</div>
                <div className="job-main">
                  <div className="job-title">
                    <strong>{jobTitle(job)}</strong>
                    <span>{Math.round(job.progress * 100)}%</span>
                  </div>
                  <div className="job-progress-line">
                    <span>{job.targetFormat}</span>
                    <div className="progress-track">
                      <div style={{ width: `${Math.round(job.progress * 100)}%` }} />
                    </div>
                    <span className="job-state">{job.status}</span>
                  </div>
                  <div className="job-meta">
                    {job.speed && <span>{job.speed}</span>}
                    {job.error && <span className="job-error">{job.error}</span>}
                  </div>
                </div>

                <div className="job-actions">
                  {job.status === 'done' ? (
                    <button className="icon-button" type="button" onClick={() => openOutput(job.outputPath)} title="Open output folder">
                      <FolderOpen size={18} />
                    </button>
                  ) : job.status === 'failed' ? (
                    <>
                      <button className="icon-button" type="button" title="Retry">
                        <RotateCcw size={17} />
                      </button>
                      <button className="icon-button danger" type="button" onClick={() => cancelJob(job.id)} title="Remove job">
                        <X size={18} />
                      </button>
                    </>
                  ) : (
                    <>
                      <button className="icon-button" type="button" onClick={job.status === 'running' ? pauseQueue : startQueue} title={job.status === 'running' ? 'Pause queue' : 'Start queue'}>
                        {job.status === 'running' ? <Pause size={17} /> : <Play size={17} />}
                      </button>
                      <button className="icon-button danger" type="button" onClick={() => cancelJob(job.id)} title="Cancel job">
                        <X size={18} />
                      </button>
                    </>
                  )}
                </div>
              </article>
            ))
          )}
        </section>
      </section>

      <aside className="control-panel">
        <div className="engine-head">
          <h2>Conversion Engine</h2>
          <p>v2.4.0-stable</p>
        </div>

        <div className="panel-scroll">
          <section>
            <h3><Cpu size={16} /> Engine Settings</h3>
            <label className="toggle-row">
              <span>Hardware Acceleration</span>
              <input
                checked={hardwareAcceleration}
                onChange={(event) => setHardwareAcceleration(event.target.checked)}
                type="checkbox"
              />
            </label>
          </section>

          <section>
            <h3><HardDrive size={16} /> Output Location</h3>
            <div className="output-card">
              <div>{outputDir || 'Same as source file'}</div>
              <button type="button" onClick={chooseOutputDir}>
                <FolderOpen size={17} /> Browse...
              </button>
            </div>
          </section>

          <section className="stats">
            <h3><Clock3 size={16} /> Progress</h3>
            <div className="meter">
              <div style={{ width: `${Math.round(summary.average * 100)}%` }} />
            </div>
            <dl>
              <div><dt>Running</dt><dd>{summary.running}</dd></div>
              <div><dt>Done</dt><dd>{summary.completed}</dd></div>
              <div><dt>Failed</dt><dd>{summary.failed}</dd></div>
            </dl>
            <button type="button" className="secondary-wide" onClick={resetQueue} disabled={!jobs.length}>Reset queue</button>
          </section>
        </div>

        <div className="panel-footer">
          <button type="button" className="start-processing" onClick={startQueue} disabled={!jobs.length || isRunning}>
            <Play size={16} /> Start Processing
          </button>
        </div>
      </aside>

      {isBatchModalOpen && activeGroup && (
        <div className="modal-backdrop" role="presentation">
          <section className="batch-modal" role="dialog" aria-modal="true" aria-labelledby="batch-title">
            <header className="modal-head">
              <div>
                <p>Batch setup</p>
                <h2 id="batch-title">Configure mixed media import</h2>
              </div>
              <button className="nav-icon" type="button" onClick={closeBatchModal} title="Close batch setup">
                <X size={20} />
              </button>
            </header>

            <div className="modal-tabs">
              {batchGroups.map((group, index) => (
                <button
                  className={index === activeGroupIndex ? 'active' : ''}
                  key={group.category}
                  onClick={() => setActiveGroupIndex(index)}
                  type="button"
                >
                  {iconFor(group.category)}
                  <span>{categoryLabel(group.category)}</span>
                  <strong>{completedGroupIndexes.includes(index) ? <Check size={14} /> : group.paths.length}</strong>
                </button>
              ))}
            </div>

            <div className="modal-body">
              <section className="modal-section">
                <h3><SlidersHorizontal size={16} /> {categoryLabel(activeGroup.category)} conversion</h3>
                <label>
                  Target Format
                  <CustomSelect
                    value={activeGroup.targetFormat}
                    options={formats[activeGroup.category].map((format) => ({ value: format, label: `.${format}` }))}
                    onChange={(value) => updateActiveGroup({
                      targetFormat: value,
                      advancedOptions: sanitizeAdvancedForFormat(activeGroup, value),
                    })}
                  />
                </label>
                <label>
                  Preset
                  <CustomSelect
                    value={activeGroup.presetName}
                    options={formats.presets.map((preset) => ({ value: preset.name, label: preset.name }))}
                    onChange={(value) => updateActiveGroup({ presetName: value })}
                  />
                </label>
                <p className="preset-description">{presetByName(activeGroup.presetName).description}</p>
                <div className="advanced-box">
                  <button
                    className="advanced-toggle"
                    type="button"
                    onClick={() => updateActiveGroup({
                      advancedOpen: !activeGroup.advancedOpen,
                      advancedOptions: ensureAdvancedOptions(activeGroup),
                    })}
                  >
                    <span>Advanced options</span>
                    <ChevronDown size={17} />
                  </button>
                  {activeGroup.advancedOpen && (
                    <div className="advanced-content">
                      <p>For experienced users. These values override the selected preset and are filtered for the chosen output format.</p>
                      {activeGroup.category !== 'image' && (
                        <label className="toggle-row">
                          <span>Copy streams / remux</span>
                          <input
                            checked={ensureAdvancedOptions(activeGroup).copyStreams}
                            onChange={(event) => updateAdvancedOptions({ copyStreams: event.target.checked })}
                            type="checkbox"
                          />
                        </label>
                      )}
                      {activeGroup.category === 'video' && !ensureAdvancedOptions(activeGroup).copyStreams && (
                        <>
                          <label>
                            Video codec
                            <CustomSelect
                              value={allowedVideoCodecs(activeGroup.targetFormat, capabilities)[0]?.id ?? ''}
                              options={allowedVideoCodecs(activeGroup.targetFormat, capabilities).map((codec) => ({ value: codec.id, label: codecLabel(codec) }))}
                              onChange={(value) => updateAdvancedOptions({ videoCodec: value })}
                            />
                          </label>
                          <label>
                            Video quality
                            <CustomSelect
                              value={String(ensureAdvancedOptions(activeGroup).videoQuality ?? 60)}
                              options={qualityLevels.map((quality) => ({ value: quality, label: qualityLabel(Number(quality)) }))}
                              onChange={(value) => updateAdvancedOptions({ videoQuality: Number(value) })}
                            />
                          </label>
                          <label>
                            Max width
                            <CustomSelect
                              value={ensureAdvancedOptions(activeGroup).maxWidth ? String(ensureAdvancedOptions(activeGroup).maxWidth) : 'Original'}
                              options={maxWidths.map((width) => ({ value: width, label: width === 'Original' ? 'Original' : `${width}px` }))}
                              onChange={(value) => updateAdvancedOptions({ maxWidth: value === 'Original' ? null : Number(value) })}
                            />
                          </label>
                        </>
                      )}
                      {(activeGroup.category === 'audio' || activeGroup.category === 'video') && !ensureAdvancedOptions(activeGroup).copyStreams && (
                        <>
                          <label>
                            Audio codec
                            <CustomSelect
                              value={allowedAudioCodecs(activeGroup.targetFormat, capabilities)[0]?.id ?? ''}
                              options={allowedAudioCodecs(activeGroup.targetFormat, capabilities).map((codec) => ({ value: codec.id, label: codecLabel(codec) }))}
                              onChange={(value) => updateAdvancedOptions({ audioCodec: value })}
                            />
                          </label>
                          {!losslessAudioOnly(activeGroup.targetFormat, capabilities) && (
                            <label>
                              Audio bitrate
                              <CustomSelect
                                value={ensureAdvancedOptions(activeGroup).audioBitrate ?? '192k'}
                                options={audioBitrates.map((bitrate) => ({ value: bitrate, label: bitrate }))}
                                onChange={(value) => updateAdvancedOptions({ audioBitrate: value })}
                              />
                            </label>
                          )}
                        </>
                      )}
                      {activeGroup.category === 'image' && (
                        <label>
                          Image quality
                          <CustomSelect
                            value={String(ensureAdvancedOptions(activeGroup).imageQuality ?? 86)}
                            options={qualityLevels.map((quality) => ({ value: quality, label: qualityLabel(Number(quality)) }))}
                            onChange={(value) => updateAdvancedOptions({ imageQuality: Number(value) })}
                          />
                        </label>
                      )}
                    </div>
                  )}
                </div>
                <div className="batch-file-list">
                  {activeGroup.paths.slice(0, 5).map((path) => <span key={path}>{fileName(path)}</span>)}
                  {activeGroup.paths.length > 5 && <span>+{activeGroup.paths.length - 5} more files</span>}
                </div>
              </section>

              <section className="modal-section">
                <h3><HardDrive size={16} /> Shared batch settings</h3>
                <div className="output-card">
                  <div>{batchOutputDir || 'Same as source file'}</div>
                  <button type="button" onClick={chooseBatchOutputDir}>
                    <FolderOpen size={17} /> Browse...
                  </button>
                </div>
              </section>
            </div>

            <footer className="modal-footer">
              <button type="button" className="secondary-wide" onClick={closeBatchModal}>Cancel</button>
              <div className="modal-step-actions">
                <button type="button" className="secondary-wide" onClick={() => setActiveGroupIndex((index) => Math.max(0, index - 1))} disabled={activeGroupIndex === 0}>
                  Back
                </button>
                {activeGroupIndex < batchGroups.length - 1 ? (
                  <button type="button" className="start-processing" onClick={continueBatchSetup}>
                    Continue
                  </button>
                ) : (
                  <button type="button" className="start-processing" onClick={confirmBatchSetup}>
                    Add to Queue
                  </button>
                )}
              </div>
            </footer>
          </section>
        </div>
      )}

      {isDocumentModalOpen && documentSetup && (
        <div className="modal-backdrop" role="presentation">
          <section className="settings-modal document-modal" role="dialog" aria-modal="true" aria-labelledby="document-title">
            <header className="modal-head">
              <div>
                <p>Document setup</p>
                <h2 id="document-title">Configure PDF task</h2>
              </div>
              <button className="nav-icon" type="button" onClick={() => setDocumentSetup(null)} title="Close document setup">
                <X size={20} />
              </button>
            </header>

            <div className="settings-body">
              <section className="modal-section">
                <h3><FileText size={16} /> Operation</h3>
                <label>
                  Document task
                  <CustomSelect
                    value={documentSetup.operation}
                    options={[
                      { value: 'imagesToPdf', label: 'Images to PDF' },
                      { value: 'mergePdfs', label: 'Merge PDFs' },
                    ]}
                    onChange={(value) => setDocumentSetup((current) => current ? {
                      ...current,
                      operation: value as DocumentOperation,
                      outputName: value === 'imagesToPdf' ? 'images.pdf' : 'merged.pdf',
                    } : current)}
                  />
                </label>
                <p className="preset-description">{documentOperationDescription(documentSetup, formats)}</p>
                <div className="batch-file-list">
                  {relevantDocumentPaths(documentSetup, formats).slice(0, 8).map((path) => <span key={path}>{fileName(path)}</span>)}
                  {irrelevantDocumentCount(documentSetup, formats) > 0 && (
                    <span>{irrelevantDocumentCount(documentSetup, formats)} file{irrelevantDocumentCount(documentSetup, formats) === 1 ? '' : 's'} ignored for this operation</span>
                  )}
                </div>
              </section>

              <section className="modal-section">
                <h3><HardDrive size={16} /> Output</h3>
                <label>
                  Output file name
                  <input
                    value={documentSetup.outputName}
                    onChange={(event) => setDocumentSetup((current) => current ? { ...current, outputName: event.target.value } : current)}
                    placeholder="merged.pdf"
                  />
                </label>
                <div className="output-card">
                  <div>{documentSetup.outputDir || 'Same as first source file'}</div>
                  <button type="button" onClick={chooseDocumentOutputDir}>
                    <FolderOpen size={17} /> Browse...
                  </button>
                </div>
              </section>
            </div>

            <footer className="modal-footer">
              <span className="settings-note">{relevantDocumentPaths(documentSetup, formats).length} input{relevantDocumentPaths(documentSetup, formats).length === 1 ? '' : 's'} ready</span>
              <button type="button" className="start-processing" onClick={confirmDocumentSetup}>
                Add to Queue
              </button>
            </footer>
          </section>
        </div>
      )}

      {isSettingsOpen && (
        <div className="modal-backdrop" role="presentation">
          <section className="settings-modal" role="dialog" aria-modal="true" aria-labelledby="settings-title">
            <header className="modal-head">
              <div>
                <p>Settings</p>
                <h2 id="settings-title">Conversion engine settings</h2>
              </div>
              <button className="nav-icon" type="button" onClick={() => setIsSettingsOpen(false)} title="Close settings">
                <X size={20} />
              </button>
            </header>

            <div className="settings-body">
              <section className="modal-section">
                <h3><Cpu size={16} /> Engine</h3>
                {capabilities.warnings.length > 0 && (
                  <div className="capability-warning">{capabilities.warnings[0]}</div>
                )}
                <div className="capability-summary">
                  <span>{capabilities.ffmpegAvailable ? 'FFmpeg detected' : 'Using fallback matrix'}</span>
                  <span>{capabilities.hardwareAccels.length ? capabilities.hardwareAccels.join(', ') : 'No hardware acceleration detected'}</span>
                </div>
                <label>
                  FFmpeg Path
                  <div className="path-picker">
                    <input value={ffmpegPath} onChange={(event) => setFfmpegPath(event.target.value)} placeholder="Bundled or system ffmpeg" />
                    <button type="button" onClick={chooseFfmpeg}>Browse</button>
                  </div>
                </label>
                <label>
                  Parallel jobs
                  <input
                    type="number"
                    min={1}
                    max={8}
                    value={parallelism}
                    onChange={(event) => setParallelism(Number(event.target.value))}
                  />
                </label>
              </section>
            </div>

            <footer className="modal-footer">
              <span className="settings-note">These settings apply when processing the queue.</span>
              <button type="button" className="start-processing" onClick={() => setIsSettingsOpen(false)}>
                Done
              </button>
            </footer>
          </section>
        </div>
      )}
    </main>
  )
}

function fileName(path: string) {
  return path.split(/[\\/]/).pop() ?? path
}

function extension(path: string) {
  return path.split('.').pop()?.toLowerCase() ?? ''
}

function isImagePath(path: string, formats: SupportedFormats) {
  return formats.image.includes(extension(path))
}

function isPdfPath(path: string, formats: SupportedFormats) {
  return formats.document.includes(extension(path))
}

function isDocumentInput(path: string, formats: SupportedFormats) {
  return isImagePath(path, formats) || isPdfPath(path, formats)
}

function relevantDocumentPaths(setup: DocumentSetup, formats: SupportedFormats) {
  return setup.operation === 'imagesToPdf'
    ? setup.paths.filter((path) => isImagePath(path, formats))
    : setup.paths.filter((path) => isPdfPath(path, formats))
}

function irrelevantDocumentCount(setup: DocumentSetup, formats: SupportedFormats) {
  return setup.paths.length - relevantDocumentPaths(setup, formats).length
}

function documentOperationDescription(setup: DocumentSetup, formats: SupportedFormats) {
  const activeCount = relevantDocumentPaths(setup, formats).length
  if (setup.operation === 'imagesToPdf') {
    return `${activeCount} image${activeCount === 1 ? '' : 's'} will become a single PDF. PDFs in this import stay available if you switch operation.`
  }
  return `${activeCount} PDF${activeCount === 1 ? '' : 's'} will be merged in the current order. Images in this import stay available if you switch operation.`
}

function jobTitle(job: ConversionJob) {
  if (job.category !== 'document') return fileName(job.inputPath)
  const count = job.inputPaths?.length ?? 1
  if (job.documentOperation === 'imagesToPdf') return `Images to PDF · ${count} file${count === 1 ? '' : 's'}`
  if (job.documentOperation === 'mergePdfs') return `Merged PDF · ${count} file${count === 1 ? '' : 's'}`
  return fileName(job.inputPath)
}

function CustomSelect({
  value,
  options,
  onChange,
}: {
  value: string
  options: Array<{ value: string; label: string }>
  onChange: (value: string) => void
}) {
  const [open, setOpen] = useState(false)
  const selected = options.find((option) => option.value === value) ?? options[0]

  return (
    <div
      className={open ? 'custom-select open' : 'custom-select'}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) {
          setOpen(false)
        }
      }}
    >
      <button type="button" onClick={() => setOpen((current) => !current)}>
        <span>{selected?.label ?? value}</span>
        <ChevronDown size={18} />
      </button>
      {open && (
        <div className="custom-select-menu">
          {options.map((option) => (
            <button
              className={option.value === value ? 'selected' : ''}
              key={option.value}
              onClick={() => {
                onChange(option.value)
                setOpen(false)
              }}
              type="button"
            >
              <span>{option.label}</span>
              {option.value === value && <Check size={14} />}
            </button>
          ))}
        </div>
      )}
    </div>
  )
}

function iconFor(category: FileCategory) {
  if (category === 'audio') return <FileAudio size={22} />
  if (category === 'image') return <FileImage size={22} />
  if (category === 'document') return <FileText size={22} />
  if (category === 'unsupported') return <AlertCircle size={22} />
  return <FileVideo size={22} />
}

function categoryLabel(category: MediaCategory) {
  const labels = {
    video: 'Video',
    audio: 'Audio',
    image: 'Images',
  }
  return labels[category]
}

function allowedVideoCodecs(targetFormat: string, capabilities: ConversionCapabilities): CodecOption[] {
  const matrix = capabilities.matrix.find((item) => item.targetFormat === targetFormat)
  const codecs = matrix?.videoCodecs.filter((codec) => codec.available) ?? []
  if (codecs.length > 0) return codecs
  return fallbackVideoCodecs(targetFormat)
}

function allowedAudioCodecs(targetFormat: string, capabilities: ConversionCapabilities): CodecOption[] {
  const matrix = capabilities.matrix.find((item) => item.targetFormat === targetFormat)
  const codecs = matrix?.audioCodecs.filter((codec) => codec.available) ?? []
  if (codecs.length > 0) return codecs
  return fallbackAudioCodecs(targetFormat)
}

function fallbackVideoCodecs(targetFormat: string): CodecOption[] {
  const ids = targetFormat === 'webm' ? ['libvpx-vp9'] : ['mpeg4']
  return ids.map((id) => ({ id, label: id, available: true, hardware: false }))
}

function fallbackAudioCodecs(targetFormat: string): CodecOption[] {
  const ids = targetFormat === 'webm' || targetFormat === 'ogg' || targetFormat === 'opus'
    ? ['libopus']
    : targetFormat === 'flac'
      ? ['flac']
      : targetFormat === 'wav'
        ? ['pcm_s16le']
        : ['aac']
  return ids.map((id) => ({ id, label: id, available: true, hardware: false }))
}

function losslessAudioOnly(targetFormat: string, capabilities: ConversionCapabilities) {
  const codecs = allowedAudioCodecs(targetFormat, capabilities)
  return codecs.length > 0 && codecs.every((codec) => codec.id === 'flac' || codec.id === 'pcm_s16le')
}

function codecLabel(codec: CodecOption) {
  return `${codec.label}${codec.hardware ? ' · GPU' : ''}`
}

function allowedBitrate(value?: string | null) {
  return value && audioBitrates.includes(value) ? value : '192k'
}

function allowedMaxWidth(value?: number | null) {
  return value && [720, 1280, 1920, 3840].includes(value) ? value : null
}

function clampQuality(value: number) {
  return Math.min(100, Math.max(1, value))
}

function qualityLabel(value: number) {
  if (value <= 20) return `${value} - smallest`
  if (value <= 40) return `${value} - compact`
  if (value <= 60) return `${value} - balanced`
  if (value <= 80) return `${value} - high`
  return `${value} - maximum`
}

export default App
