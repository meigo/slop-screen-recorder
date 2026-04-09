<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { open } from '@tauri-apps/plugin-dialog';
	import { onMount } from 'svelte';

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

	let ffmpegAvailable = $state(false);
	let sources = $state<Source[]>([]);
	let audioDevices = $state<Source[]>([]);
	let selectedSource = $state('');
	let selectedAudio = $state<string | null>(null);
	let captureAudio = $state(false);
	let outputDir = $state('');
	let fps = $state(30);
	let recording = $state(false);
	let outputPath = $state('');
	let elapsed = $state(0);
	let timer: ReturnType<typeof setInterval> | null = null;

	onMount(async () => {
		ffmpegAvailable = await invoke<boolean>('check_ffmpeg');
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
		} catch (e) {
			alert(`Failed to stop recording: ${e}`);
		}
	}

	async function pickOutputDir() {
		const selected = await open({ directory: true, multiple: false });
		if (selected) outputDir = selected as string;
	}
</script>

<main>
	<h1>Slop Screen Recorder</h1>

	{#if !ffmpegAvailable}
		<div class="error">
			<p>FFmpeg not found. Please install FFmpeg to use this app.</p>
			<p class="hint">macOS: <code>brew install ffmpeg</code></p>
			<p class="hint">Windows: Download from ffmpeg.org and add to PATH</p>
		</div>
	{:else}
		<div class="controls">
			<div class="field">
				<label for="source">Source</label>
				<select id="source" bind:value={selectedSource} disabled={recording}>
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
		font-family:
			-apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell,
			sans-serif;
		background: #1a1a2e;
		color: #eee;
	}

	main {
		max-width: 480px;
		margin: 0 auto;
		padding: 2rem;
	}

	h1 {
		text-align: center;
		font-size: 1.4rem;
		font-weight: 600;
		margin-bottom: 2rem;
		color: #fff;
	}

	.error {
		background: #2d1b1b;
		border: 1px solid #5c2b2b;
		border-radius: 8px;
		padding: 1.5rem;
		text-align: center;
	}

	.error p {
		margin: 0.5rem 0;
	}

	.hint {
		font-size: 0.85rem;
		color: #999;
	}

	code {
		background: #2a2a3e;
		padding: 0.2rem 0.5rem;
		border-radius: 4px;
		font-size: 0.85rem;
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
		font-size: 0.85rem;
		color: #aaa;
	}

	.field.row label {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		color: #eee;
		cursor: pointer;
	}

	select,
	input[type='text'] {
		background: #2a2a3e;
		border: 1px solid #3a3a5e;
		border-radius: 6px;
		padding: 0.5rem 0.75rem;
		color: #eee;
		font-size: 0.9rem;
	}

	select:focus,
	input:focus {
		outline: none;
		border-color: #6366f1;
	}

	.dir-picker {
		display: flex;
		gap: 0.5rem;
	}

	.dir-picker input {
		flex: 1;
	}

	.dir-picker button {
		background: #2a2a3e;
		border: 1px solid #3a3a5e;
		border-radius: 6px;
		padding: 0.5rem 1rem;
		color: #eee;
		cursor: pointer;
		font-size: 0.85rem;
	}

	.dir-picker button:hover {
		background: #3a3a5e;
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
		border-radius: 12px;
		padding: 0.85rem 2.5rem;
		font-size: 1rem;
		font-weight: 600;
		cursor: pointer;
		transition: all 0.15s;
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
		background: #3a3a5e;
		color: white;
	}

	.btn.stop:hover {
		background: #4a4a6e;
	}

	.output-info {
		margin-top: 1.5rem;
		background: #1e3a2e;
		border: 1px solid #2d5a3e;
		border-radius: 8px;
		padding: 1rem;
		text-align: center;
	}

	.output-info p {
		margin: 0 0 0.5rem 0;
		font-size: 0.85rem;
		color: #aaa;
	}

	.output-info code {
		word-break: break-all;
	}

	input[type='checkbox'] {
		accent-color: #6366f1;
	}
</style>
