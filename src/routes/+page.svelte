<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { getCurrentWindow } from '@tauri-apps/api/window';
	import { open } from '@tauri-apps/plugin-dialog';
	import { register, unregister } from '@tauri-apps/plugin-global-shortcut';
	import { onMount, onDestroy } from 'svelte';

	interface Source {
		id: string;
		name: string;
		source_type: string;
	}

	interface RecordingConfig {
		source_id: string;
		output_dir: string;
		fps: number;
		capture_audio: boolean;
		audio_device: string | null;
	}

	let loading = $state(true);
	let ffmpegAvailable = $state(false);
	let sources = $state<Source[]>([]);
	let audioDevices = $state<Source[]>([]);
	let selectedSource = $state('');
	let selectedAudio = $state<string | null>(null);
	let captureAudio = $state(false);
	let outputDir = $state('');
	let fps = $state(30);
	let minimizeOnRecord = $state(false);
	let recording = $state(false);
	let outputPath = $state('');
	let elapsed = $state(0);
	let timer: ReturnType<typeof setInterval> | null = null;

	onMount(async () => {
		ffmpegAvailable = await invoke<boolean>('check_ffmpeg');
		loading = false;
		if (!ffmpegAvailable) return;

		const [srcs, audio, defaultDir] = await Promise.all([
			invoke<Source[]>('list_sources'),
			invoke<Source[]>('list_audio_devices'),
			invoke<string>('get_default_output_dir'),
		]);

		sources = srcs;
		audioDevices = audio;
		outputDir = defaultDir;

		// Auto-select first screen source
		const screen = sources.find((s) => s.source_type === 'screen');
		if (screen) selectedSource = screen.id;

		if (audioDevices.length > 0) selectedAudio = audioDevices[0].id;

		await register('CommandOrControl+Shift+R', async (event) => {
			if (event.state === 'Released') return;
			if (recording) {
				await stopRecording();
			} else {
				await startRecording();
			}
		});
	});

	onDestroy(async () => {
		try {
			await unregister('CommandOrControl+Shift+R');
		} catch {
			// ignore cleanup errors
		}
	});

	function formatTime(seconds: number): string {
		const m = Math.floor(seconds / 60)
			.toString()
			.padStart(2, '0');
		const s = (seconds % 60).toString().padStart(2, '0');
		return `${m}:${s}`;
	}

	async function startRecording() {
		const config: RecordingConfig = {
			source_id: selectedSource,
			output_dir: outputDir,
			fps,
			capture_audio: captureAudio,
			audio_device: captureAudio ? selectedAudio : null,
		};

		try {
			outputPath = await invoke<string>('start_recording', { config });
			recording = true;
			elapsed = 0;
			timer = setInterval(() => elapsed++, 1000);
			if (minimizeOnRecord) {
				await getCurrentWindow().minimize();
			}
		} catch (e) {
			alert(`Failed to start recording: ${e}`);
		}
	}

	async function stopRecording() {
		try {
			const path = await invoke<string>('stop_recording');
			recording = false;
			if (timer) {
				clearInterval(timer);
				timer = null;
			}
			outputPath = path;
			await getCurrentWindow().unminimize();
		} catch (e) {
			alert(`Failed to stop recording: ${e}`);
		}
	}

	async function refreshSources() {
		const srcs = await invoke<Source[]>('list_sources');
		sources = srcs;
		// If selected source no longer exists, fall back to first screen
		if (!sources.find((s) => s.id === selectedSource)) {
			const screen = sources.find((s) => s.source_type === 'screen');
			selectedSource = screen ? screen.id : sources[0]?.id ?? '';
		}
	}

	async function pickOutputDir() {
		const selected = await open({ directory: true, multiple: false });
		if (selected) outputDir = selected as string;
	}
</script>

<main>
	<h1>Slop Screen Recorder</h1>

	{#if loading}
		<!-- wait for init -->
	{:else if !ffmpegAvailable}
		<div class="error">
			<p>FFmpeg not found.</p>
			<p class="hint">Install FFmpeg and make sure it's available in your PATH.</p>
		</div>
	{:else}
		<div class="controls">
			<div class="field">
				<label for="source">Source</label>
				<select id="source" bind:value={selectedSource} disabled={recording} onfocus={refreshSources}>
					{#each sources as source (source.id)}
						<option value={source.id}>{source.name}</option>
					{/each}
				</select>
			</div>

			<div class="field">
				<label for="fps">FPS</label>
				<select id="fps" bind:value={fps} disabled={recording}>
					<option value={15}>15</option>
					<option value={24}>24</option>
					<option value={25}>25</option>
					<option value={30}>30</option>
					<option value={60}>60</option>
				</select>
			</div>

			<div class="field row">
				<label>
					<input type="checkbox" bind:checked={captureAudio} disabled={recording} />
					Capture audio
				</label>
			</div>

			<div class="field row">
				<label>
					<input type="checkbox" bind:checked={minimizeOnRecord} disabled={recording} />
					Minimize on record
				</label>
			</div>

			{#if captureAudio && audioDevices.length > 0}
				<div class="field">
					<label for="audio">Audio device</label>
					<select id="audio" bind:value={selectedAudio} disabled={recording}>
						{#each audioDevices as device (device.id)}
							<option value={device.id}>{device.name}</option>
						{/each}
					</select>
				</div>
			{/if}

			<div class="field">
				<label for="output">Output directory</label>
				<div class="dir-picker">
					<input id="output" type="text" bind:value={outputDir} readonly />
					<button onclick={pickOutputDir} disabled={recording}>Browse</button>
				</div>
			</div>
		</div>

		<div class="record-section">
			{#if recording}
				<div class="timer">{formatTime(elapsed)}</div>
				<button class="btn stop" onclick={stopRecording}>Stop Recording</button>
			{:else}
				<button class="btn record" onclick={startRecording} disabled={!selectedSource}>
					Start Recording
				</button>
			{/if}
		</div>

		{#if outputPath && !recording}
			<div class="output-info">
				<p>Saved to:</p>
				<code>{outputPath}</code>
			</div>
		{/if}
	{/if}
</main>

<style>
	:global(body) {
		margin: 0;
		font-family: 'JetBrains Mono', monospace;
		background: #1a1a1a;
		color: #ffffff;
	}

	main {
		max-width: 480px;
		margin: 0 auto;
		padding: 2rem;
	}

	h1 {
		text-align: center;
		font-size: 1.3rem;
		font-weight: 700;
		margin-bottom: 2rem;
		color: #ffffff;
		letter-spacing: -0.02em;
	}

	.error {
		background: #2a2020;
		border: 1px solid #443030;
		border-radius: 8px;
		padding: 1.5rem;
		text-align: center;
	}

	.error p {
		margin: 0.5rem 0;
	}

	.hint {
		font-size: 0.8rem;
		color: #888;
	}

	code {
		background: #2a2a2a;
		padding: 0.2rem 0.5rem;
		border-radius: 4px;
		font-size: 0.8rem;
		font-family: 'JetBrains Mono', monospace;
	}

	.controls {
		display: flex;
		flex-direction: column;
		gap: 1rem;
	}

	.field {
		display: flex;
		flex-direction: column;
		gap: 0.3rem;
	}

	.field.row {
		flex-direction: row;
		align-items: center;
	}

	label {
		font-size: 0.8rem;
		color: #888;
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}

	.field.row label {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		color: #ccc;
		text-transform: none;
		letter-spacing: normal;
		cursor: pointer;
	}

	select,
	input[type='text'] {
		background: #2a2a2a;
		border: 1px solid #3a3a3a;
		border-radius: 6px;
		padding: 0.5rem 0.75rem;
		color: #ffffff;
		font-size: 0.85rem;
		font-family: 'JetBrains Mono', monospace;
	}

	select:focus,
	input:focus {
		outline: none;
		border-color: #666;
	}

	.dir-picker {
		display: flex;
		gap: 0.5rem;
	}

	.dir-picker input {
		flex: 1;
	}

	.dir-picker button {
		background: #2a2a2a;
		border: 1px solid #3a3a3a;
		border-radius: 6px;
		padding: 0.5rem 1rem;
		color: #ffffff;
		cursor: pointer;
		font-size: 0.8rem;
		font-family: 'JetBrains Mono', monospace;
	}

	.dir-picker button:hover {
		background: #333;
	}

	.record-section {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 1rem;
		margin-top: 2rem;
	}

	.timer {
		font-size: 2.5rem;
		font-weight: 700;
		font-variant-numeric: tabular-nums;
		color: #ef4444;
	}

	.btn {
		border: none;
		border-radius: 8px;
		padding: 0.85rem 2.5rem;
		font-size: 0.9rem;
		font-weight: 600;
		cursor: pointer;
		transition: all 0.15s;
		font-family: 'JetBrains Mono', monospace;
	}

	.btn:disabled {
		opacity: 0.4;
		cursor: not-allowed;
	}

	.btn.record {
		background: #ef4444;
		color: white;
	}

	.btn.record:hover:not(:disabled) {
		background: #dc2626;
		transform: scale(1.03);
	}

	.btn.stop {
		background: #333;
		color: white;
	}

	.btn.stop:hover {
		background: #444;
	}

	.output-info {
		margin-top: 1.5rem;
		background: #1e2e1e;
		border: 1px solid #2a3e2a;
		border-radius: 8px;
		padding: 1rem;
		text-align: center;
	}

	.output-info p {
		margin: 0 0 0.5rem 0;
		font-size: 0.8rem;
		color: #888;
	}

	.output-info code {
		word-break: break-all;
	}

	input[type='checkbox'] {
		accent-color: #ef4444;
	}
</style>
