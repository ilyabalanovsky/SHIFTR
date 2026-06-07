import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
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
  Pencil,
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
type UserLevel = 'aware' | 'capable' | 'fluent'
type ReleaseStatus = {
  latestVersion: string | null
  updateAvailable: boolean
}

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
  frameRate?: number | null
  targetSizeMb?: number | null
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
  processingSeconds?: number | null
  etaSeconds?: number | null
  error?: string | null
  errorDetails?: string | null
}

type EncodingPreset = {
  id: string
  name: string
  description: string
  category: FileCategory
  platform?: string | null
  targetFormat: string
  preset: ConversionPreset
  advancedOptions?: AdvancedOptions | null
  builtIn: boolean
}

type SizeTargetValidation = {
  applicable: boolean
  warnings: string[]
  estimates: Array<{
    path: string
    durationSeconds?: number | null
    totalKbps?: number | null
    videoKbps?: number | null
    audioKbps?: number | null
    applicable: boolean
    warning?: string | null
  }>
}

type JobTooltipState = {
  jobId: string
  left: number
  top: number
  placement: 'below' | 'above'
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
  presetOverride: ConversionPreset | null
  encodingPresetId: string | null
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
const frameRates = ['Same as source', '24', '25', '30', '50', '60', '120']
const targetSizes = ['Keep auto', '8', '10', '25', '50', '100', '250', '500', '1024', 'Custom...']
const fixedTargetSizes = [8, 10, 25, 50, 100, 250, 500, 1024]
const qualityLevels = ['20', '40', '60', '80', '95']
const userLevelStorageKey = 'shiftr.userLevel'
const jobTooltipDelayMs = 1500
const appVersion = __APP_VERSION__
const githubReleasesRepo = __GITHUB_RELEASES_REPO__

function App() {
  const [userLevel, setUserLevelState] = useState<UserLevel>(() => storedUserLevel() ?? 'capable')
  const [isOnboardingOpen, setIsOnboardingOpen] = useState(() => storedUserLevel() === null)
  const [activeMode, setActiveMode] = useState<AppMode>('media')
  const [formats, setFormats] = useState<SupportedFormats>(fallbackFormats)
  const [capabilities, setCapabilities] = useState<ConversionCapabilities>(fallbackCapabilities)
  const [jobs, setJobs] = useState<ConversionJob[]>([])
  const [targetFormat, setTargetFormat] = useState('mp4')
  const [presetName, setPresetName] = useState('Balanced')
  const [outputDir, setOutputDir] = useState('')
  const [parallelism, setParallelism] = useState(2)
  const [ffmpegPath, setFfmpegPath] = useState('')
  const [isRunning, setIsRunning] = useState(false)
  const [dragActive, setDragActive] = useState(false)
  const [lastError, setLastError] = useState<string | null>(null)
  const [isSettingsOpen, setIsSettingsOpen] = useState(false)
  const [isPresetsOpen, setIsPresetsOpen] = useState(false)
  const [encodingPresets, setEncodingPresets] = useState<EncodingPreset[]>([])
  const [customPresetName, setCustomPresetName] = useState('')
  const [batchGroups, setBatchGroups] = useState<BatchGroup[]>([])
  const [activeGroupIndex, setActiveGroupIndex] = useState(0)
  const [completedGroupIndexes, setCompletedGroupIndexes] = useState<number[]>([])
  const [batchOutputDir, setBatchOutputDir] = useState('')
  const [documentSetup, setDocumentSetup] = useState<DocumentSetup | null>(null)
  const [renameDraft, setRenameDraft] = useState<{ id: string; value: string } | null>(null)
  const [releaseStatus, setReleaseStatus] = useState<ReleaseStatus | null>(null)
  const [expandedErrorIds, setExpandedErrorIds] = useState<string[]>([])
  const [customSizeCategories, setCustomSizeCategories] = useState<MediaCategory[]>([])
  const [sizeTargetValidation, setSizeTargetValidation] = useState<{ key: string; value: SizeTargetValidation } | null>(null)
  const [jobTooltip, setJobTooltip] = useState<JobTooltipState | null>(null)
  const jobTooltipTimerRef = useRef<number | null>(null)

  const selectedPreset = useMemo(
    () => formats.presets.find((preset) => preset.name === presetName) ?? formats.presets[0] ?? fallbackPreset,
    [formats.presets, presetName],
  )

  const summary = useMemo(() => {
    const completed = jobs.filter((job) => job.status === 'done').length
    const failed = jobs.filter((job) => job.status === 'failed').length
    const running = jobs.filter((job) => job.status === 'running').length
    const queued = jobs.filter((job) => job.status === 'queued').length
    const average = jobs.length ? jobs.reduce((sum, job) => sum + job.progress, 0) / jobs.length : 0
    const completedDurations = jobs
      .filter((job) => job.status === 'done' && job.processingSeconds != null && job.processingSeconds > 0)
      .map((job) => job.processingSeconds ?? 0)
    const runningProjectedDurations = jobs
      .filter((job) => job.status === 'running' && job.processingSeconds != null && job.progress > 0.02)
      .map((job) => Math.ceil((job.processingSeconds ?? 0) / job.progress))
    const knownDurations = completedDurations.length ? completedDurations : runningProjectedDurations
    const averageJobSeconds = knownDurations.length
      ? knownDurations.reduce((sum, seconds) => sum + seconds, 0) / knownDurations.length
      : null
    const runningEtas = jobs
      .filter((job) => job.status === 'running' && job.etaSeconds != null)
      .map((job) => job.etaSeconds ?? 0)
    const runningEta = runningEtas.length ? Math.max(...runningEtas) : null
    const queuedEta = averageJobSeconds ? Math.ceil(queued / Math.max(1, parallelism)) * averageJobSeconds : null
    const queueEta = runningEta != null || queuedEta != null
      ? Math.ceil((runningEta ?? 0) + (queuedEta ?? 0))
      : null
    return { completed, failed, running, average, queueEta }
  }, [jobs, parallelism])

  const activeGroup = batchGroups[activeGroupIndex]
  const isBatchModalOpen = batchGroups.length > 0
  const isDocumentModalOpen = documentSetup !== null
  const tooltipJob = jobTooltip ? jobs.find((job) => job.id === jobTooltip.jobId) : null

  const queueOptions = useCallback(() => ({
    outputDir: outputDir || null,
    targetFormat,
    preset: selectedPreset,
    advancedOptions: null,
    parallelism,
    ffmpegPath: ffmpegPath || null,
  }), [ffmpegPath, outputDir, parallelism, selectedPreset, targetFormat])

  useEffect(() => () => clearJobTooltipTimer(), [])

  useEffect(() => {
    invoke<SupportedFormats>('get_supported_formats')
      .then((value) => {
        setFormats(value)
        setParallelism(value.defaultParallelism)
      })
      .catch(() => setFormats(fallbackFormats))

    refreshEncodingPresets()

    const unlistenPromise = listen<QueueUpdate>('queue://job-updated', (event) => {
      setJobs((current) =>
        current.map((job) => (job.id === event.payload.job.id ? event.payload.job : job)),
      )
    })

    return () => {
      unlistenPromise.then((unlisten) => unlisten()).catch(() => undefined)
    }
  }, [])

  function refreshEncodingPresets() {
    invoke<EncodingPreset[]>('get_encoding_presets')
      .then(setEncodingPresets)
      .catch((error) => setLastError(String(error)))
  }

  useEffect(() => {
    invoke<ConversionCapabilities>('get_conversion_capabilities', { ffmpegPath: ffmpegPath || null })
      .then(setCapabilities)
      .catch((error) => setCapabilities({ ...fallbackCapabilities, warnings: [String(error)] }))
  }, [ffmpegPath])

  useEffect(() => {
    if (!githubReleasesRepo) return

    const controller = new AbortController()
    fetch(`https://api.github.com/repos/${githubReleasesRepo}/releases/latest`, {
      signal: controller.signal,
      headers: { Accept: 'application/vnd.github+json' },
    })
      .then((response) => response.ok ? response.json() as Promise<{ tag_name?: string; name?: string }> : null)
      .then((release) => {
        const latestVersion = normalizeVersion(release?.tag_name ?? release?.name ?? '')
        if (!latestVersion) return
        setReleaseStatus({
          latestVersion,
          updateAvailable: compareVersions(latestVersion, appVersion) > 0,
        })
      })
      .catch(() => undefined)

    return () => controller.abort()
  }, [])

  useEffect(() => {
    if (!activeGroup || !['video', 'audio'].includes(activeGroup.category)) {
      return
    }

    const targetSizeMb = activeGroup.advancedOptions?.targetSizeMb
    if (!targetSizeMb) {
      return
    }

    const validationKey = sizeTargetValidationKey(activeGroup)
    let canceled = false
    invoke<SizeTargetValidation>('validate_size_target', {
      request: {
        paths: activeGroup.paths,
        category: activeGroup.category,
        targetSizeMb,
        audioBitrate: activeGroup.advancedOptions?.audioBitrate ?? null,
        ffmpegPath: ffmpegPath || null,
      },
    })
      .then((validation) => {
        if (!canceled) setSizeTargetValidation({ key: validationKey, value: validation })
      })
      .catch((error) => {
        if (!canceled) {
          setSizeTargetValidation({
            key: validationKey,
            value: {
              applicable: false,
              warnings: [String(error)],
              estimates: [],
            },
          })
        }
      })

    return () => {
      canceled = true
    }
  }, [activeGroup, ffmpegPath])

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
      .map((category) => {
        const recipe = userLevel === 'aware'
          ? compatibleEncodingPresets(encodingPresets, category)[0]
          : undefined
        return {
          category,
          paths: buckets[category],
          targetFormat: recipe?.targetFormat ?? preferredTarget(category),
          presetName: recipe?.preset.name ?? presetName,
          presetOverride: recipe?.preset ?? null,
          encodingPresetId: recipe?.id ?? null,
          advancedOpen: userLevel === 'fluent',
          advancedOptions: recipe?.advancedOptions ?? (userLevel === 'fluent' ? null : null),
        }
      })
  }, [categoryForPath, encodingPresets, preferredTarget, presetName, userLevel])

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

  function resetOutputDirToSource() {
    setOutputDir('')
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
        preset: group.presetOverride ?? presetByName(group.presetName),
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

  function updateActiveGroup(update: Partial<Pick<BatchGroup, 'targetFormat' | 'presetName' | 'presetOverride' | 'encodingPresetId' | 'advancedOpen' | 'advancedOptions'>>) {
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
        audioCodec: allowedCodecId(options.audioCodec, allowedAudioCodecs(group.targetFormat, capabilities)),
        audioBitrate: losslessAudioOnly(group.targetFormat, capabilities) 
          ? null
          : allowedBitrate(options.audioBitrate),
        targetSizeMb: allowedTargetSizeMb(options.targetSizeMb),
      }
    }
    return {
      copyStreams: options.copyStreams,
      videoCodec: allowedCodecId(options.videoCodec, allowedVideoCodecs(group.targetFormat, capabilities)),
      audioCodec: allowedCodecId(options.audioCodec, allowedAudioCodecs(group.targetFormat, capabilities)),
      videoQuality: clampQuality(options.videoQuality ?? 60),
      audioBitrate: losslessAudioOnly(group.targetFormat, capabilities)
        ? null
        : allowedBitrate(options.audioBitrate),
      maxWidth: allowedMaxWidth(options.maxWidth),
      frameRate: allowedFrameRate(options.frameRate),
      targetSizeMb: allowedTargetSizeMb(options.targetSizeMb),
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
      frameRate: null,
      targetSizeMb: null,
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

  async function retryJob(id: string) {
    setLastError(null)
    try {
      const retried = await invoke<ConversionJob>('retry_job', { id })
      setExpandedErrorIds((current) => current.filter((item) => item !== id))
      setJobs((current) => current.map((job) => (job.id === id ? retried : job)))
    } catch (error) {
      setLastError(String(error))
    }
  }

  async function removeJob(id: string) {
    setLastError(null)
    try {
      const remaining = await invoke<ConversionJob[]>('remove_job', { id })
      setExpandedErrorIds((current) => current.filter((item) => item !== id))
      setJobs(remaining)
    } catch (error) {
      setLastError(String(error))
    }
  }

  function startRename(job: ConversionJob) {
    if (job.status !== 'queued') return
    setRenameDraft({ id: job.id, value: fileName(job.outputPath) })
  }

  async function confirmRename() {
    if (!renameDraft) return
    setLastError(null)
    try {
      const updated = await invoke<ConversionJob>('rename_job_output', {
        options: {
          id: renameDraft.id,
          outputName: renameDraft.value,
        },
      })
      setJobs((current) => current.map((job) => (job.id === updated.id ? updated : job)))
      setRenameDraft(null)
    } catch (error) {
      setLastError(String(error))
    }
  }

  async function openOutput(path: string) {
    await invoke('open_output_folder', { path })
  }

  async function clearDone() {
    setLastError(null)
    try {
      const remaining = await invoke<ConversionJob[]>('clear_finished_jobs')
      setExpandedErrorIds((current) => current.filter((id) => remaining.some((job) => job.id === id)))
      setJobs(remaining)
    } catch (error) {
      setLastError(String(error))
    }
  }

  async function resetQueue() {
    setLastError(null)
    try {
      await invoke('reset_queue')
      setExpandedErrorIds([])
      setJobs([])
    } catch (error) {
      setLastError(String(error))
    }
  }

  function toggleErrorDetails(id: string) {
    setExpandedErrorIds((current) =>
      current.includes(id) ? current.filter((item) => item !== id) : [...current, id],
    )
  }

  function clearJobTooltipTimer() {
    if (jobTooltipTimerRef.current !== null) {
      window.clearTimeout(jobTooltipTimerRef.current)
      jobTooltipTimerRef.current = null
    }
  }

  function scheduleJobTooltip(jobId: string, element: HTMLElement) {
    clearJobTooltipTimer()
    jobTooltipTimerRef.current = window.setTimeout(() => {
      jobTooltipTimerRef.current = null
      if (element.isConnected) {
        showJobTooltip(jobId, element)
      }
    }, jobTooltipDelayMs)
  }

  function showJobTooltip(jobId: string, element: HTMLElement) {
    const rect = element.getBoundingClientRect()
    const width = Math.min(380, window.innerWidth - 32)
    const estimatedHeight = 210
    const left = Math.min(Math.max(rect.left + 58, 16), Math.max(16, window.innerWidth - width - 16))
    const hasRoomBelow = rect.bottom + estimatedHeight + 18 <= window.innerHeight
    const top = hasRoomBelow
      ? Math.min(rect.bottom + 10, window.innerHeight - estimatedHeight - 16)
      : Math.max(16, rect.top - estimatedHeight - 10)

    setJobTooltip({
      jobId,
      left,
      top,
      placement: hasRoomBelow ? 'below' : 'above',
    })
  }

  function hideJobTooltip() {
    clearJobTooltipTimer()
    setJobTooltip(null)
  }

  function setUserLevel(level: UserLevel) {
    localStorage.setItem(userLevelStorageKey, level)
    setUserLevelState(level)
    setIsOnboardingOpen(false)
    setBatchGroups((current) =>
      current.map((group) => ({
        ...group,
        advancedOpen: level === 'fluent' ? true : level === 'aware' ? false : group.advancedOpen,
      })),
    )
  }

  function applyEncodingPreset(preset: EncodingPreset) {
    applyEncodingPresetToBatch(preset, true)
  }

  function applyEncodingPresetToBatch(preset: EncodingPreset, closePresets: boolean) {
    const compatibleGroupIndex = activeGroup?.category === preset.category
      ? activeGroupIndex
      : batchGroups.findIndex((group) => group.category === preset.category)

    if (compatibleGroupIndex < 0) {
      setLastError(`Add ${preset.category} files before applying this preset.`)
      return
    }

    setBatchGroups((current) =>
      current.map((group, index) => index === compatibleGroupIndex
        ? {
          ...group,
          targetFormat: preset.targetFormat,
          presetName: preset.preset.name,
          presetOverride: preset.preset,
          encodingPresetId: preset.id,
          advancedOpen: Boolean(preset.advancedOptions),
          advancedOptions: preset.advancedOptions ?? null,
        }
        : group),
    )
    setActiveGroupIndex(compatibleGroupIndex)
    if (closePresets) setIsPresetsOpen(false)
    setLastError(null)
  }

  async function saveActiveGroupAsPreset() {
    if (!activeGroup) {
      setLastError('Add files before saving a preset.')
      return
    }
    const name = customPresetName.trim()
    if (!name) {
      setLastError('Enter a preset name first.')
      return
    }

    const preset: EncodingPreset = {
      id: `custom_${Date.now()}`,
      name,
      description: `Custom ${categoryLabel(activeGroup.category).toLowerCase()} recipe.`,
      category: activeGroup.category,
      platform: 'Custom',
      targetFormat: activeGroup.targetFormat,
      preset: activeGroup.presetOverride ?? presetByName(activeGroup.presetName),
      advancedOptions: normalizedAdvancedOptions({
        ...activeGroup,
        advancedOpen: true,
        advancedOptions: ensureAdvancedOptions(activeGroup),
      }),
      builtIn: false,
    }

    try {
      const updated = await invoke<EncodingPreset[]>('save_custom_encoding_preset', { preset })
      setEncodingPresets(updated)
      setCustomPresetName('')
      setLastError(null)
    } catch (error) {
      setLastError(String(error))
    }
  }

  async function deleteEncodingPreset(id: string) {
    try {
      const updated = await invoke<EncodingPreset[]>('delete_custom_encoding_preset', { id })
      setEncodingPresets(updated)
      setLastError(null)
    } catch (error) {
      setLastError(String(error))
    }
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
          <button className="nav-icon" type="button" onClick={() => setIsPresetsOpen(true)} title="Encoding presets">
            <SlidersHorizontal size={22} />
          </button>
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
              <article
                className={`job-row ${job.status}`}
                key={job.id}
                onBlur={hideJobTooltip}
                onFocus={(event) => scheduleJobTooltip(job.id, event.currentTarget)}
                onMouseEnter={(event) => scheduleJobTooltip(job.id, event.currentTarget)}
                onMouseLeave={hideJobTooltip}
                tabIndex={0}
              >
                <div className="job-icon">{iconFor(job.category)}</div>
                <div className="job-main">
                  <div className="job-title">
                    <div className="job-name-line">
                      {renameDraft?.id === job.id ? (
                        <div className="rename-inline">
                          <input
                            autoFocus
                            value={renameDraft.value}
                            onChange={(event) => setRenameDraft({ ...renameDraft, value: event.target.value })}
                            onKeyDown={(event) => {
                              if (event.key === 'Enter') void confirmRename()
                              if (event.key === 'Escape') setRenameDraft(null)
                            }}
                            aria-label="Output file name"
                          />
                          <button className="icon-button" type="button" onClick={confirmRename} title="Save output name">
                            <Check size={15} />
                          </button>
                          <button className="icon-button danger" type="button" onClick={() => setRenameDraft(null)} title="Cancel rename">
                            <X size={15} />
                          </button>
                        </div>
                      ) : (
                        <>
                          <strong>{jobTitle(job)}</strong>
                          {job.status === 'queued' && (
                            <button className="rename-button" type="button" onClick={() => startRename(job)} title="Rename output file">
                              <Pencil size={14} />
                            </button>
                          )}
                        </>
                      )}
                    </div>
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
                    {job.processingSeconds != null && <span>{formatProcessingTime(job.processingSeconds)}</span>}
                    {job.status === 'running' && job.etaSeconds != null && (
                      <span>ETA {formatProcessingTime(job.etaSeconds)}</span>
                    )}
                    {job.error && <span className="job-error">{job.error}</span>}
                    {job.errorDetails && (
                      <button className="details-toggle" type="button" onClick={() => toggleErrorDetails(job.id)}>
                        {expandedErrorIds.includes(job.id) ? 'Hide details' : 'Technical details'}
                      </button>
                    )}
                  </div>
                  {job.errorDetails && expandedErrorIds.includes(job.id) && (
                    <pre className="error-details">{job.errorDetails}</pre>
                  )}
                </div>

                <div className="job-actions">
                  {job.status === 'done' ? (
                    <button className="icon-button" type="button" onClick={() => openOutput(job.outputPath)} title="Open output folder">
                      <FolderOpen size={18} />
                    </button>
                  ) : job.status === 'failed' ? (
                    <>
                      <button className="icon-button" type="button" onClick={() => retryJob(job.id)} title="Retry">
                        <RotateCcw size={17} />
                      </button>
                      <button className="icon-button danger" type="button" onClick={() => removeJob(job.id)} title="Remove job">
                        <X size={18} />
                      </button>
                    </>
                  ) : job.status === 'canceled' ? (
                    <>
                      <button className="icon-button" type="button" onClick={() => retryJob(job.id)} title="Retry">
                        <RotateCcw size={17} />
                      </button>
                      <button className="icon-button danger" type="button" onClick={() => removeJob(job.id)} title="Remove job">
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

      {tooltipJob && jobTooltip && (
        <div
          className={`job-tooltip ${jobTooltip.placement}`}
          role="tooltip"
          style={{
            left: jobTooltip.left,
            top: jobTooltip.top,
          }}
        >
          <div className="job-tooltip-head">
            <span>{fileCategoryLabel(tooltipJob.category)} setup</span>
            <strong>{jobTitle(tooltipJob)}</strong>
          </div>
          <dl>
            {jobSettingsRows(tooltipJob).map((row) => (
              <div key={row.label}>
                <dt>{row.label}</dt>
                <dd title={row.value}>{row.value}</dd>
              </div>
            ))}
          </dl>
        </div>
      )}

      <aside className="control-panel">
        <div className="engine-head">
          <h2>Conversion Engine</h2>
          <p>
            v{appVersion}
            {releaseStatus && (
              <span className={releaseStatus.updateAvailable ? 'update-available' : ''}>
                {releaseStatus.updateAvailable ? `Latest v${releaseStatus.latestVersion} available` : 'Up to date'}
              </span>
            )}
          </p>
        </div>

        <div className="panel-scroll">
          <section className="stats">
            <h3><Clock3 size={16} /> Progress</h3>
            <div className="meter">
              <div style={{ width: `${Math.round(summary.average * 100)}%` }} />
            </div>
            <div className="queue-eta">
              <span>Estimated time</span>
              <strong>{summary.queueEta != null ? formatProcessingTime(summary.queueEta) : 'Estimating...'}</strong>
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
                {userLevel === 'aware' ? (
                  <div className="recipe-first">
                    <label>
                      Ready-made recipe
                      <CustomSelect
                        value={activeGroup.encodingPresetId ?? compatibleEncodingPresets(encodingPresets, activeGroup.category)[0]?.id ?? ''}
                        options={compatibleEncodingPresets(encodingPresets, activeGroup.category).map((preset) => ({ value: preset.id, label: `${preset.name} · .${preset.targetFormat}` }))}
                        onChange={(value) => {
                          const recipe = encodingPresets.find((preset) => preset.id === value)
                          if (recipe) applyEncodingPresetToBatch(recipe, false)
                        }}
                      />
                    </label>
                    <p className="preset-description">
                      {activeRecipe(activeGroup, encodingPresets)?.description ?? 'Choose a recipe and SHIFTR will handle format, codec, quality, and size settings.'}
                    </p>
                    <button className="secondary-wide" type="button" onClick={() => setIsPresetsOpen(true)}>
                      Browse all recipes
                    </button>
                  </div>
                ) : (
                  <>
                    <label>
                      Target Format
                      <CustomSelect
                        value={activeGroup.targetFormat}
                        options={formats[activeGroup.category].map((format) => ({ value: format, label: `.${format}` }))}
                        onChange={(value) => updateActiveGroup({
                          targetFormat: value,
                          encodingPresetId: null,
                          presetOverride: null,
                          advancedOptions: sanitizeAdvancedForFormat(activeGroup, value),
                        })}
                      />
                    </label>
                    <label>
                      Preset
                      <CustomSelect
                        value={activeGroup.presetName}
                        options={presetSelectOptions(activeGroup, formats)}
                        onChange={(value) => updateActiveGroup({ presetName: value, presetOverride: null, encodingPresetId: null })}
                      />
                    </label>
                    <p className="preset-description">{activeGroup.presetOverride?.description ?? presetByName(activeGroup.presetName).description}</p>
                    <div className="advanced-disclosure">
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
                              value={allowedCodecId(ensureAdvancedOptions(activeGroup).videoCodec, allowedVideoCodecs(activeGroup.targetFormat, capabilities)) ?? ''}
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
                          <label>
                            Frame rate
                            <CustomSelect
                              value={ensureAdvancedOptions(activeGroup).frameRate ? String(ensureAdvancedOptions(activeGroup).frameRate) : 'Same as source'}
                              options={frameRates.map((rate) => ({ value: rate, label: rate === 'Same as source' ? rate : `${rate} fps` }))}
                              onChange={(value) => updateAdvancedOptions({ frameRate: value === 'Same as source' ? null : Number(value) })}
                            />
                          </label>
                        </>
                      )}
                      {(activeGroup.category === 'audio' || activeGroup.category === 'video') && !ensureAdvancedOptions(activeGroup).copyStreams && (
                        <>
                          <label>
                            Audio codec
                            <CustomSelect
                              value={allowedCodecId(ensureAdvancedOptions(activeGroup).audioCodec, allowedAudioCodecs(activeGroup.targetFormat, capabilities)) ?? ''}
                              options={allowedAudioCodecs(activeGroup.targetFormat, capabilities).map((codec) => ({ value: codec.id, label: codecLabel(codec) }))}
                              onChange={(value) => updateAdvancedOptions({ audioCodec: value })}
                            />
                          </label>
                          {!losslessAudioOnly(activeGroup.targetFormat, capabilities) && (
                            <>
                              <label>
                                Audio bitrate
                                <CustomSelect
                                  value={ensureAdvancedOptions(activeGroup).audioBitrate ?? '192k'}
                                  options={audioBitrates.map((bitrate) => ({ value: bitrate, label: bitrate }))}
                                  onChange={(value) => updateAdvancedOptions({ audioBitrate: value })}
                                />
                              </label>
                              <label>
                                Size target
                                <CustomSelect
                                  value={targetSizeSelectValue(activeGroup, customSizeCategories)}
                                  options={targetSizes.map((size) => ({ value: size, label: targetSizeLabel(size) }))}
                                  onChange={(value) => {
                                    if (value === 'Keep auto') {
                                      setCustomSizeCategories((current) => current.filter((category) => category !== activeGroup.category))
                                      updateAdvancedOptions({ targetSizeMb: null })
                                    } else if (value === 'Custom...') {
                                      setCustomSizeCategories((current) => Array.from(new Set([...current, activeGroup.category])))
                                      updateAdvancedOptions({ targetSizeMb: ensureAdvancedOptions(activeGroup).targetSizeMb ?? 26 })
                                    } else {
                                      setCustomSizeCategories((current) => current.filter((category) => category !== activeGroup.category))
                                      updateAdvancedOptions({ targetSizeMb: Number(value) })
                                    }
                                  }}
                                />
                              </label>
                              {isCustomSizeTarget(activeGroup, customSizeCategories) && (
                                <label>
                                  Custom size, MB
                                  <input
                                    min={1}
                                    max={10240}
                                    type="number"
                                    value={ensureAdvancedOptions(activeGroup).targetSizeMb ?? 26}
                                    onChange={(event) => updateAdvancedOptions({ targetSizeMb: clampTargetSize(Number(event.target.value)) })}
                                  />
                                </label>
                              )}
                              {ensureAdvancedOptions(activeGroup).targetSizeMb != null && sizeTargetValidation?.key === sizeTargetValidationKey(activeGroup) && (
                                <div className={sizeTargetValidation.value.applicable ? 'size-target-note' : 'size-target-note warning'}>
                                  {sizeTargetValidation.value.warnings.length > 0
                                    ? sizeTargetValidation.value.warnings.slice(0, 3).map((warning) => <span key={warning}>{warning}</span>)
                                    : <span>{sizeTargetSummary(sizeTargetValidation.value)}</span>}
                                </div>
                              )}
                            </>
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
                  </>
                )}
              </section>

              <section className="modal-section">
                <h3><HardDrive size={16} /> Output location</h3>
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

      {isPresetsOpen && (
        <div className="modal-backdrop" role="presentation">
          <section className="settings-modal presets-modal" role="dialog" aria-modal="true" aria-labelledby="presets-title">
            <header className="modal-head">
              <div>
                <p>Presets</p>
                <h2 id="presets-title">Encoding recipes</h2>
              </div>
              <button className="nav-icon" type="button" onClick={() => setIsPresetsOpen(false)} title="Close presets">
                <X size={20} />
              </button>
            </header>

            <div className="settings-body presets-body">
              {activeGroup && (
                <section className="modal-section">
                  <h3><Check size={16} /> Save current setup</h3>
                  <div className="path-picker">
                    <input
                      value={customPresetName}
                      onChange={(event) => setCustomPresetName(event.target.value)}
                      placeholder={`${categoryLabel(activeGroup.category)} preset name`}
                    />
                    <button type="button" onClick={saveActiveGroupAsPreset}>Save</button>
                  </div>
                </section>
              )}

              <section className="modal-section">
                <h3><SlidersHorizontal size={16} /> Built-in and custom presets</h3>
                <div className="preset-grid">
                  {encodingPresets.filter((preset) => isMediaCategory(preset.category)).map((preset) => (
                    <article className="preset-card" key={preset.id}>
                      <div>
                        <span>{preset.platform ?? (preset.builtIn ? 'Built-in' : 'Custom')}</span>
                        <strong>{preset.name}</strong>
                        <p>{preset.description}</p>
                      </div>
                      <div className="preset-card-meta">
                        <span>{categoryLabel(preset.category as MediaCategory)} · .{preset.targetFormat}</span>
                        <span>{preset.builtIn ? 'Built-in' : 'Custom'}</span>
                      </div>
                      <div className="preset-actions">
                        <button type="button" className="secondary-wide" onClick={() => applyEncodingPreset(preset)}>
                          Apply
                        </button>
                        {!preset.builtIn && (
                          <button type="button" className="icon-button danger" onClick={() => deleteEncodingPreset(preset.id)} title="Delete preset">
                            <X size={17} />
                          </button>
                        )}
                      </div>
                    </article>
                  ))}
                </div>
              </section>
            </div>

            <footer className="modal-footer">
              <span className="settings-note">Presets apply to the matching batch tab.</span>
              <button type="button" className="start-processing" onClick={() => setIsPresetsOpen(false)}>
                Done
              </button>
            </footer>
          </section>
        </div>
      )}

      {isOnboardingOpen && (
        <div className="modal-backdrop" role="presentation">
          <section className="onboarding-modal" role="dialog" aria-modal="true" aria-labelledby="onboarding-title">
            <header className="modal-head">
              <div>
                <p>First run</p>
                <h2 id="onboarding-title">How hands-on do you want SHIFTR to be?</h2>
              </div>
            </header>

            <div className="onboarding-grid">
              {(['aware', 'capable', 'fluent'] as const).map((level) => (
                <button
                  className={`onboarding-card ${level}`}
                  key={level}
                  onClick={() => setUserLevel(level)}
                  type="button"
                >
                  <span>{userLevelEyebrow(level)}</span>
                  <strong>{userLevelTitle(level)}</strong>
                  <p>{userLevelDescription(level)}</p>
                </button>
              ))}
            </div>
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
                <h3><SlidersHorizontal size={16} /> Experience level</h3>
                <CustomSelect
                  value={userLevel}
                  options={[
                    { value: 'aware', label: 'Aware' },
                    { value: 'capable', label: 'Capable' },
                    { value: 'fluent', label: 'Fluent' },
                  ]}
                  onChange={(value) => setUserLevel(value as UserLevel)}
                />
                <p className="preset-description">{userLevelDescription(userLevel)}</p>
              </section>

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

              <section className="modal-section">
                <h3><HardDrive size={16} /> Default output directory</h3>
                <label className="toggle-row">
                  <span>Same as source file</span>
                  <input
                    checked={!outputDir}
                    onChange={(event) => {
                      if (event.target.checked) resetOutputDirToSource()
                      else void chooseOutputDir()
                    }}
                    type="checkbox"
                  />
                </label>
                {outputDir ? (
                  <div className="output-card">
                    <div>{outputDir}</div>
                    <button type="button" onClick={chooseOutputDir}>
                      <FolderOpen size={17} /> Browse...
                    </button>
                  </div>
                ) : (
                  <button type="button" className="secondary-wide" onClick={chooseOutputDir}>
                    <FolderOpen size={17} /> Choose folder
                  </button>
                )}
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

function isMediaCategory(category: FileCategory): category is MediaCategory {
  return category === 'video' || category === 'audio' || category === 'image'
}

function storedUserLevel(): UserLevel | null {
  const value = localStorage.getItem(userLevelStorageKey)
  return value === 'aware' || value === 'capable' || value === 'fluent' ? value : null
}

function userLevelTitle(level: UserLevel) {
  if (level === 'aware') return 'Aware'
  if (level === 'fluent') return 'Fluent'
  return 'Capable'
}

function userLevelEyebrow(level: UserLevel) {
  if (level === 'aware') return 'Preset guided'
  if (level === 'fluent') return 'Full control'
  return 'Hands-on'
}

function userLevelDescription(level: UserLevel) {
  if (level === 'aware') {
    return 'Use ready-made recipes, import shared presets, and let SHIFTR choose the technical settings.'
  }
  if (level === 'fluent') {
    return 'Open advanced controls by default and tune codecs, bitrate, frame rate, size targets, and more.'
  }
  return 'Choose formats and quality presets yourself, with advanced controls available when you need them.'
}

function compatibleEncodingPresets(presets: EncodingPreset[], category: MediaCategory) {
  return presets.filter((preset) => preset.category === category)
}

function activeRecipe(group: BatchGroup, presets: EncodingPreset[]) {
  return presets.find((preset) => preset.id === group.encodingPresetId)
}

function jobTitle(job: ConversionJob) {
  return fileName(job.outputPath)
}

function jobSettingsRows(job: ConversionJob) {
  const options = job.advancedOptions
  const rows = [
    { label: 'Format', value: `.${job.sourceFormat || 'unknown'} -> .${job.targetFormat}` },
    { label: 'Preset', value: `${job.preset.name} · ${qualityModeLabel(job.preset.qualityMode)}` },
  ]

  if (job.category === 'video') {
    rows.push(
      { label: 'Video', value: compactCodecValue(options?.copyStreams, options?.videoCodec) },
      { label: 'Audio', value: compactCodecValue(options?.copyStreams, options?.audioCodec) },
      { label: 'Quality', value: options?.videoQuality ? qualityLabel(options.videoQuality) : 'Preset decides' },
      { label: 'Limits', value: compactVideoLimits(options) },
      { label: 'Size', value: options?.targetSizeMb ? formatTargetSize(options.targetSizeMb) : 'Keep auto' },
    )
  } else if (job.category === 'audio') {
    rows.push(
      { label: 'Audio', value: compactCodecValue(options?.copyStreams, options?.audioCodec) },
      { label: 'Bitrate', value: options?.targetSizeMb ? 'From size target' : options?.audioBitrate ?? 'Preset decides' },
      { label: 'Size', value: options?.targetSizeMb ? formatTargetSize(options.targetSizeMb) : 'Keep auto' },
    )
  } else if (job.category === 'image') {
    rows.push(
      { label: 'Quality', value: options?.imageQuality ? qualityLabel(options.imageQuality) : 'Preset decides' },
    )
  } else if (job.category === 'document') {
    rows.push(
      { label: 'Operation', value: documentJobLabel(job.documentOperation) },
      { label: 'Inputs', value: `${job.inputPaths.length} file${job.inputPaths.length === 1 ? '' : 's'}` },
    )
  }

  rows.push({ label: 'Status', value: compactJobTiming(job) })

  return rows
}

function compactCodecValue(copyStreams?: boolean | null, codec?: string | null) {
  if (copyStreams) return 'Copy source'
  return codec || 'Preset decides'
}

function compactVideoLimits(options?: AdvancedOptions | null) {
  const limits = []
  if (options?.maxWidth) limits.push(`${options.maxWidth}px`)
  if (options?.frameRate) limits.push(`${options.frameRate} fps`)
  return limits.length > 0 ? limits.join(' · ') : 'Original'
}

function compactJobTiming(job: ConversionJob) {
  const elapsed = job.processingSeconds != null ? formatProcessingTime(job.processingSeconds) : null
  if (job.status === 'running' && job.etaSeconds != null) {
    return `${elapsed ?? 'Running'} · ETA ${formatProcessingTime(job.etaSeconds)}`
  }
  if (elapsed) return `${job.status} · ${elapsed}`
  return job.status
}

function fileCategoryLabel(category: FileCategory) {
  if (category === 'video') return 'Video'
  if (category === 'audio') return 'Audio'
  if (category === 'image') return 'Image'
  if (category === 'document') return 'Document'
  return 'File'
}

function qualityModeLabel(mode: QualityMode) {
  if (mode === 'fastRemux') return 'Fast remux'
  if (mode === 'fastEncode') return 'Fast encode'
  if (mode === 'smallSize') return 'Small size'
  if (mode === 'highQuality') return 'High quality'
  if (mode === 'keepSource') return 'Keep source quality'
  return 'Balanced'
}

function formatTargetSize(sizeMb: number) {
  return sizeMb >= 1024 ? `${Math.round(sizeMb / 1024)} GB` : `${sizeMb} MB`
}

function documentJobLabel(operation?: DocumentOperation | null) {
  if (operation === 'imagesToPdf') return 'Images to PDF'
  if (operation === 'mergePdfs') return 'Merge PDFs'
  return 'Document task'
}

function formatProcessingTime(seconds: number) {
  const total = Math.max(0, Math.floor(seconds))
  const hours = Math.floor(total / 3600)
  const minutes = Math.floor((total % 3600) / 60)
  const secs = total % 60

  if (hours > 0) {
    return minutes > 0 ? `${hours} hr ${minutes} min` : `${hours} hr`
  }
  if (minutes > 0) {
    return secs > 0 ? `${minutes} min ${secs} sec` : `${minutes} min`
  }
  return `${secs} sec`
}

function normalizeVersion(value: string) {
  return value.trim().replace(/^v/i, '')
}

function compareVersions(left: string, right: string) {
  const leftParts = normalizeVersion(left).split(/[.-]/).map((part) => Number.parseInt(part, 10) || 0)
  const rightParts = normalizeVersion(right).split(/[.-]/).map((part) => Number.parseInt(part, 10) || 0)
  const maxLength = Math.max(leftParts.length, rightParts.length)

  for (let index = 0; index < maxLength; index += 1) {
    const diff = (leftParts[index] ?? 0) - (rightParts[index] ?? 0)
    if (diff !== 0) return diff
  }
  return 0
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

function presetSelectOptions(group: BatchGroup, formats: SupportedFormats) {
  const options = formats.presets.map((preset) => ({ value: preset.name, label: preset.name }))
  if (group.presetOverride && !options.some((option) => option.value === group.presetOverride?.name)) {
    return [{ value: group.presetOverride.name, label: group.presetOverride.name }, ...options]
  }
  return options
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

function allowedCodecId(value: string | null | undefined, codecs: CodecOption[]) {
  return codecs.find((codec) => codec.id === value)?.id ?? codecs[0]?.id
}

function allowedBitrate(value?: string | null) {
  return value && audioBitrates.includes(value) ? value : '192k'
}

function allowedMaxWidth(value?: number | null) {
  return value && [720, 1280, 1920, 3840].includes(value) ? value : null
}

function allowedFrameRate(value?: number | null) {
  return value && [24, 25, 30, 50, 60, 120].includes(value) ? value : null
}

function allowedTargetSizeMb(value?: number | null) {
  return value && value >= 1 && value <= 10240 ? Math.round(value) : null
}

function clampTargetSize(value: number) {
  if (!Number.isFinite(value)) return 1
  return Math.min(10240, Math.max(1, Math.round(value)))
}

function targetSizeLabel(size: string) {
  if (size === 'Keep auto' || size === 'Custom...') return size
  return size === '1024' ? '1 GB' : `${size} MB`
}

function targetSizeSelectValue(group: BatchGroup, customSizeCategories: MediaCategory[]) {
  const value = group.advancedOptions?.targetSizeMb
  if (!value) return 'Keep auto'
  if (customSizeCategories.includes(group.category) || !fixedTargetSizes.includes(value)) return 'Custom...'
  return String(value)
}

function isCustomSizeTarget(group: BatchGroup, customSizeCategories: MediaCategory[]) {
  const value = group.advancedOptions?.targetSizeMb
  return Boolean(value && (customSizeCategories.includes(group.category) || !fixedTargetSizes.includes(value)))
}

function sizeTargetValidationKey(group: BatchGroup) {
  return [
    group.category,
    group.targetFormat,
    group.advancedOptions?.targetSizeMb ?? 'auto',
    group.advancedOptions?.audioBitrate ?? 'audio-auto',
    group.paths.join('|'),
  ].join(':')
}

function sizeTargetSummary(validation: SizeTargetValidation) {
  const first = validation.estimates.find((estimate) => estimate.totalKbps != null)
  if (!first) return 'SHIFTR will validate this target when duration is available.'
  if (first.videoKbps != null) {
    return `Estimated budget: ${first.videoKbps}k video + ${first.audioKbps ?? 0}k audio.`
  }
  return `Estimated audio bitrate: ${first.audioKbps ?? first.totalKbps}k.`
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
