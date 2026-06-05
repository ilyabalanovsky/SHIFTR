import { useCallback, useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'
import {
  CheckCircle2,
  Clock3,
  FileAudio,
  FileImage,
  FileVideo,
  FolderOpen,
  Pause,
  Play,
  RotateCcw,
  Settings2,
  Square,
  Upload,
  XCircle,
} from 'lucide-react'
import './App.css'

type FileCategory = 'video' | 'audio' | 'image' | 'unsupported'
type JobStatus = 'queued' | 'running' | 'done' | 'failed' | 'canceled'
type QualityMode = 'smallSize' | 'balanced' | 'highQuality' | 'keepSource'
type OverwritePolicy = 'rename' | 'overwrite'

type ConversionPreset = {
  name: string
  qualityMode: QualityMode
  overwritePolicy: OverwritePolicy
}

type ConversionJob = {
  id: string
  inputPath: string
  outputPath: string
  sourceFormat: string
  targetFormat: string
  category: FileCategory
  preset: ConversionPreset
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
  presets: ConversionPreset[]
  defaultParallelism: number
}

type QueueUpdate = {
  job: ConversionJob
}

const fallbackPreset: ConversionPreset = {
  name: 'Balanced',
  qualityMode: 'balanced',
  overwritePolicy: 'rename',
}

const fallbackFormats: SupportedFormats = {
  video: ['mp4', 'mkv', 'mov', 'webm', 'avi'],
  audio: ['mp3', 'aac', 'm4a', 'ogg', 'opus', 'wav', 'flac'],
  image: ['png', 'jpg', 'jpeg', 'webp', 'bmp', 'tiff'],
  presets: [
    fallbackPreset,
    { name: 'Small size', qualityMode: 'smallSize', overwritePolicy: 'rename' },
    { name: 'High quality', qualityMode: 'highQuality', overwritePolicy: 'rename' },
    { name: 'Keep source quality', qualityMode: 'keepSource', overwritePolicy: 'rename' },
  ],
  defaultParallelism: 2,
}

function App() {
  const [formats, setFormats] = useState<SupportedFormats>(fallbackFormats)
  const [jobs, setJobs] = useState<ConversionJob[]>([])
  const [targetFormat, setTargetFormat] = useState('mp4')
  const [presetName, setPresetName] = useState('Balanced')
  const [outputDir, setOutputDir] = useState('')
  const [parallelism, setParallelism] = useState(2)
  const [ffmpegPath, setFfmpegPath] = useState('')
  const [isRunning, setIsRunning] = useState(false)
  const [dragActive, setDragActive] = useState(false)
  const [lastError, setLastError] = useState<string | null>(null)

  const selectedPreset = useMemo(
    () => formats.presets.find((preset) => preset.name === presetName) ?? formats.presets[0] ?? fallbackPreset,
    [formats.presets, presetName],
  )

  const allFormats = useMemo(
    () => [
      ...formats.video.map((format) => ({ format, category: 'Video' })),
      ...formats.audio.map((format) => ({ format, category: 'Audio' })),
      ...formats.image.map((format) => ({ format, category: 'Image' })),
    ],
    [formats],
  )

  const summary = useMemo(() => {
    const completed = jobs.filter((job) => job.status === 'done').length
    const failed = jobs.filter((job) => job.status === 'failed').length
    const running = jobs.filter((job) => job.status === 'running').length
    const average = jobs.length ? jobs.reduce((sum, job) => sum + job.progress, 0) / jobs.length : 0
    return { completed, failed, running, average }
  }, [jobs])

  const hasQueuedJobs = useMemo(() => jobs.some((job) => job.status === 'queued'), [jobs])

  const queueOptions = useCallback(() => ({
    outputDir: outputDir || null,
    targetFormat,
    preset: selectedPreset,
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
    if (!hasQueuedJobs) return

    const timeout = window.setTimeout(() => {
      invoke<ConversionJob[]>('update_queued_jobs', { options: queueOptions() })
        .then(setJobs)
        .catch((error) => setLastError(String(error)))
    }, 120)

    return () => window.clearTimeout(timeout)
  }, [hasQueuedJobs, queueOptions])

  async function addPaths(paths: string[]) {
    if (!paths.length) return
    setLastError(null)
    try {
      const created = await invoke<ConversionJob[]>('create_jobs', {
        paths,
        options: queueOptions(),
      })
      setJobs(created)
    } catch (error) {
      setLastError(String(error))
    }
  }

  async function chooseFiles() {
    const selected = await open({
      multiple: true,
      filters: [
        {
          name: 'Media and images',
          extensions: [...formats.video, ...formats.audio, ...formats.image],
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

  async function chooseFfmpeg() {
    const selected = await open({ multiple: false })
    if (typeof selected === 'string') {
      setFfmpegPath(selected)
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
      <section className="workspace">
        <header className="topbar">
          <div>
            <p className="eyebrow">Local media conversion</p>
            <h1>SHIFTR</h1>
          </div>
          <div className="topbar-actions">
            <button className="icon-button" type="button" onClick={chooseOutputDir} title="Choose output folder">
              <FolderOpen size={18} />
            </button>
            <button className="icon-button" type="button" onClick={chooseFfmpeg} title="Set FFmpeg path">
              <Settings2 size={18} />
            </button>
          </div>
        </header>

        <div
          className={dragActive ? 'dropzone active' : 'dropzone'}
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
          <Upload size={24} />
          <div>
            <strong>Drop files here</strong>
            <span>Video, audio, and images stay on this device.</span>
          </div>
          <button type="button" onClick={chooseFiles}>Choose files</button>
        </div>

        {lastError && <div className="error-strip">{lastError}</div>}

        <section className="queue-header">
          <div>
            <h2>Queue</h2>
            <p>{jobs.length ? `${jobs.length} item${jobs.length === 1 ? '' : 's'} ready` : 'No files selected'}</p>
          </div>
          <div className="queue-actions">
            <button type="button" onClick={clearDone} disabled={!jobs.length}>
              <RotateCcw size={16} /> Clear finished
            </button>
            <button type="button" className="primary" onClick={startQueue} disabled={!jobs.length || isRunning}>
              <Play size={16} /> Start
            </button>
            <button type="button" onClick={pauseQueue} disabled={!isRunning}>
              <Pause size={16} /> Pause
            </button>
          </div>
        </section>

        <section className="queue-list">
          {jobs.length === 0 ? (
            <div className="empty-state">
              <FileVideo size={26} />
              <span>Select files to build a conversion queue.</span>
            </div>
          ) : (
            jobs.map((job) => (
              <article className="job-row" key={job.id}>
                <div className="job-icon">{iconFor(job.category)}</div>
                <div className="job-main">
                  <div className="job-title">
                    <strong>{fileName(job.inputPath)}</strong>
                    <span>{job.sourceFormat || 'file'} to {job.targetFormat}</span>
                  </div>
                  <div className="progress-track">
                    <div style={{ width: `${Math.round(job.progress * 100)}%` }} />
                  </div>
                  <div className="job-meta">
                    <span>{job.outputPath}</span>
                    {job.speed && <span>{job.speed}</span>}
                    {job.error && <span className="job-error">{job.error}</span>}
                  </div>
                </div>
                <div className={`status-pill ${job.status}`}>{statusLabel(job.status)}</div>
                <button className="icon-button" type="button" onClick={() => openOutput(job.outputPath)} title="Open output folder">
                  <FolderOpen size={16} />
                </button>
                <button className="icon-button danger" type="button" onClick={() => cancelJob(job.id)} title="Cancel job">
                  <Square size={16} />
                </button>
              </article>
            ))
          )}
        </section>
      </section>

      <aside className="control-panel">
        <section>
          <h2>Format</h2>
          <label>
            Target
            <select value={targetFormat} onChange={(event) => setTargetFormat(event.target.value)}>
              {allFormats.map((item) => (
                <option key={`${item.category}-${item.format}`} value={item.format}>
                  {item.category} .{item.format}
                </option>
              ))}
            </select>
          </label>
          <label>
            Preset
            <select value={presetName} onChange={(event) => setPresetName(event.target.value)}>
              {formats.presets.map((preset) => (
                <option key={preset.name} value={preset.name}>{preset.name}</option>
              ))}
            </select>
          </label>
        </section>

        <section>
          <h2>Output</h2>
          <label>
            Folder
            <input value={outputDir} onChange={(event) => setOutputDir(event.target.value)} placeholder="Same as source" />
          </label>
          <label>
            FFmpeg
            <input value={ffmpegPath} onChange={(event) => setFfmpegPath(event.target.value)} placeholder="Bundled or system ffmpeg" />
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

        <section className="stats">
          <h2>Progress</h2>
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
      </aside>
    </main>
  )
}

function fileName(path: string) {
  return path.split(/[\\/]/).pop() ?? path
}

function iconFor(category: FileCategory) {
  if (category === 'audio') return <FileAudio size={20} />
  if (category === 'image') return <FileImage size={20} />
  if (category === 'unsupported') return <XCircle size={20} />
  return <FileVideo size={20} />
}

function statusLabel(status: JobStatus) {
  const labels: Record<JobStatus, string> = {
    queued: 'Queued',
    running: 'Running',
    done: 'Done',
    failed: 'Failed',
    canceled: 'Canceled',
  }
  return (
    <>
      {status === 'done' && <CheckCircle2 size={14} />}
      {status === 'queued' && <Clock3 size={14} />}
      {status === 'failed' && <XCircle size={14} />}
      {labels[status]}
    </>
  )
}

export default App
