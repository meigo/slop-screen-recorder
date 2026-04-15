<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { getCurrentWindow, currentMonitor } from '@tauri-apps/api/window';
	import { open } from '@tauri-apps/plugin-dialog';
	import { open as openInShell } from '@tauri-apps/plugin-shell';
	import { register, unregister } from '@tauri-apps/plugin-global-shortcut';
	import { onMount, onDestroy } from 'svelte';

	interface Source {
		id: string;
		name: string;
		source_type: string;
	}

	type Region = {
		x: number;
		y: number;
		width: number;
		height: number;
	};

	interface RecordingConfig {
		source_id: string;
		output_dir: string;
		fps: number;
		capture_audio: boolean;
		audio_device: string | null;
		region: Region | null;
	}

	const REGION_PRESETS: Record<string, { w: number; h: number }> = {
		// Landscape (16:9)
		'854x480': { w: 854, h: 480 },
		'1280x720': { w: 1280, h: 720 },
		'1920x1080': { w: 1920, h: 1080 },
		'2560x1440': { w: 2560, h: 1440 },
		'3840x2160': { w: 3840, h: 2160 },
		// Portrait (9:16)
		'480x854': { w: 480, h: 854 },
		'720x1280': { w: 720, h: 1280 },
		'1080x1920': { w: 1080, h: 1920 },
		'1440x2560': { w: 1440, h: 2560 },
		'2160x3840': { w: 2160, h: 3840 },
		// Square (1:1)
		'720x720': { w: 720, h: 720 },
		'1080x1080': { w: 1080, h: 1080 },
		'1440x1440': { w: 1440, h: 1440 },
	};


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
	let useRegion = $state(false);
	let regionPreset = $state('1920x1080');
	let regionX = $state(0);
	let regionY = $state(0);
	let screenSize = $state<{ w: number; h: number } | null>(null);
	let overlayVisible = $state(false);

	const selectedSourceType = $derived(
		sources.find((s) => s.id === selectedSource)?.source_type ?? 'screen',
	);
	const canUseRegion = $derived(selectedSourceType === 'screen' && screenSize !== null);

	const previewRegion = $derived.by(() => {
		if (!useRegion || !screenSize) return null;
		return computeRegion();
	});
	// Fit a preview box inside a max 280×160 area while preserving aspect ratio.
	const previewBox = $derived.by(() => {
		if (!screenSize) return null;
		const maxW = 280;
		const maxH = 160;
		const scale = Math.min(maxW / screenSize.w, maxH / screenSize.h);
		return { w: screenSize.w * scale, h: screenSize.h * scale, scale };
	});

	function currentRegionSize(): { w: number; h: number } | null {
		if (!screenSize) return null;
		const preset = REGION_PRESETS[regionPreset];
		if (!preset) return null;
		return {
			w: Math.min(preset.w, screenSize.w) & ~1,
			h: Math.min(preset.h, screenSize.h) & ~1,
		};
	}

	function clampPosition(x: number, y: number, w: number, h: number): { x: number; y: number } {
		if (!screenSize) return { x: 0, y: 0 };
		return {
			x: Math.max(0, Math.min(screenSize.w - w, Math.round(x))),
			y: Math.max(0, Math.min(screenSize.h - h, Math.round(y))),
		};
	}

	function centerRegion() {
		const size = currentRegionSize();
		if (!size) return;
		const pos = clampPosition((screenSize!.w - size.w) / 2, (screenSize!.h - size.h) / 2, size.w, size.h);
		regionX = pos.x;
		regionY = pos.y;
	}

	function computeRegion(): Region | null {
		if (!useRegion || !canUseRegion) return null;
		const size = currentRegionSize();
		if (!size) return null;
		const pos = clampPosition(regionX, regionY, size.w, size.h);
		return { x: pos.x, y: pos.y, width: size.w, height: size.h };
	}

	// Re-clamp x/y whenever preset size or screen changes so the rect stays on-screen.
	$effect(() => {
		const size = currentRegionSize();
		if (!size) return;
		const pos = clampPosition(regionX, regionY, size.w, size.h);
		if (pos.x !== regionX) regionX = pos.x;
		if (pos.y !== regionY) regionY = pos.y;
	});

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

		const monitor = await currentMonitor();
		if (monitor) {
			screenSize = { w: monitor.size.width, h: monitor.size.height };
			centerRegion();
		}

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
			region: computeRegion(),
		};

		try {
			outputPath = await invoke<string>('start_recording', { config });
			recording = true;
			overlayVisible = false; // backend closes it; sync local state
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

	async function openVideo() {
		try {
			await openInShell(outputPath);
		} catch (e) {
			alert(`Failed to open video: ${e}`);
		}
	}

	let dragStart: { mouseX: number; mouseY: number; regionX: number; regionY: number } | null = null;

	function onRectPointerDown(e: PointerEvent) {
		if (recording) return;
		const target = e.currentTarget as SVGElement;
		target.setPointerCapture(e.pointerId);
		dragStart = { mouseX: e.clientX, mouseY: e.clientY, regionX, regionY };
	}

	function onRectPointerMove(e: PointerEvent) {
		if (!dragStart || !previewBox) return;
		const dxPreview = e.clientX - dragStart.mouseX;
		const dyPreview = e.clientY - dragStart.mouseY;
		const dxScreen = dxPreview / previewBox.scale;
		const dyScreen = dyPreview / previewBox.scale;
		const size = currentRegionSize();
		if (!size) return;
		const pos = clampPosition(dragStart.regionX + dxScreen, dragStart.regionY + dyScreen, size.w, size.h);
		regionX = pos.x;
		regionY = pos.y;
	}

	function onRectPointerUp(e: PointerEvent) {
		const target = e.currentTarget as SVGElement;
		if (target.hasPointerCapture(e.pointerId)) target.releasePointerCapture(e.pointerId);
		dragStart = null;
	}

	async function toggleOverlay() {
		if (overlayVisible) {
			await invoke('hide_region_overlay');
			overlayVisible = false;
			return;
		}
		const region = computeRegion();
		if (!region) return;
		await invoke('show_region_overlay', region);
		overlayVisible = true;
	}

	// Keep overlay in sync with region preset/position changes while it's visible.
	$effect(() => {
		if (!overlayVisible) return;
		if (!previewRegion) return;
		invoke('show_region_overlay', previewRegion).catch(() => {});
	});

	// Close overlay if source changes away from a screen source.
	$effect(() => {
		if (!canUseRegion && overlayVisible) {
			invoke('hide_region_overlay').catch(() => {});
			overlayVisible = false;
		}
	});

	async function showInFolder() {
		const parent = outputPath.replace(/[/\\][^/\\]*$/, '');
		try {
			await openInShell(parent);
		} catch (e) {
			alert(`Failed to open folder: ${e}`);
		}
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

			{#if canUseRegion}
				<div class="field row">
					<label>
						<input type="checkbox" bind:checked={useRegion} disabled={recording} />
						Record region only
					</label>
				</div>

				{#if useRegion}
					<div class="field">
						<label for="region-size">Region size</label>
						<select id="region-size" bind:value={regionPreset} disabled={recording}>
							<optgroup label="Landscape (16:9)">
								<option value="854x480">480p (854×480)</option>
								<option value="1280x720">720p (1280×720)</option>
								<option value="1920x1080">1080p (1920×1080)</option>
								<option value="2560x1440">1440p (2560×1440)</option>
								<option value="3840x2160">4K (3840×2160)</option>
							</optgroup>
							<optgroup label="Portrait (9:16)">
								<option value="480x854">480×854</option>
								<option value="720x1280">720×1280</option>
								<option value="1080x1920">1080×1920</option>
								<option value="1440x2560">1440×2560</option>
								<option value="2160x3840">2160×3840</option>
							</optgroup>
							<optgroup label="Square (1:1)">
								<option value="720x720">720×720</option>
								<option value="1080x1080">1080×1080</option>
								<option value="1440x1440">1440×1440</option>
							</optgroup>
						</select>
					</div>

					{#if previewBox && previewRegion && screenSize}
						<div class="region-preview">
							<svg
								width={previewBox.w}
								height={previewBox.h}
								viewBox={`0 0 ${previewBox.w} ${previewBox.h}`}
							>
								<rect
									class="screen-rect"
									x="0"
									y="0"
									width={previewBox.w}
									height={previewBox.h}
								/>
								<rect
									class="region-rect"
									class:dragging={dragStart !== null}
									x={previewRegion.x * previewBox.scale}
									y={previewRegion.y * previewBox.scale}
									width={previewRegion.width * previewBox.scale}
									height={previewRegion.height * previewBox.scale}
									onpointerdown={onRectPointerDown}
									onpointermove={onRectPointerMove}
									onpointerup={onRectPointerUp}
									onpointercancel={onRectPointerUp}
								/>
							</svg>
							<div class="region-caption">
								Screen {screenSize.w}×{screenSize.h} · Region {previewRegion.width}×{previewRegion.height}
								@ ({previewRegion.x},{previewRegion.y})
							</div>
							<div class="region-actions">
								<button class="preview-refresh" onclick={centerRegion} disabled={recording}>
									Center
								</button>
								<button class="preview-refresh" onclick={toggleOverlay} disabled={recording}>
									{overlayVisible ? 'Hide layout overlay' : 'Show layout overlay'}
								</button>
							</div>
						</div>
					{/if}
				{/if}
			{/if}

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
				<div class="output-actions">
					<button onclick={openVideo}>Open video</button>
					<button onclick={showInFolder}>Show in folder</button>
				</div>
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

	.output-actions {
		display: flex;
		gap: 0.5rem;
		justify-content: center;
		margin-top: 0.75rem;
	}

	.output-actions button {
		background: #2a2a2a;
		border: 1px solid #3a3a3a;
		border-radius: 6px;
		padding: 0.4rem 0.9rem;
		color: #ffffff;
		cursor: pointer;
		font-size: 0.75rem;
		font-family: 'JetBrains Mono', monospace;
	}

	.output-actions button:hover {
		background: #333;
	}

	input[type='checkbox'] {
		accent-color: #ef4444;
	}

	.region-preview {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 0.5rem;
		padding: 0.75rem;
		background: #222;
		border: 1px solid #333;
		border-radius: 6px;
	}

	.region-preview svg {
		display: block;
	}

	.region-preview .screen-rect {
		fill: #1a1a1a;
		stroke: #444;
		stroke-width: 1;
	}

	.region-preview .region-rect {
		fill: rgba(239, 68, 68, 0.25);
		stroke: #ef4444;
		stroke-width: 1;
		cursor: grab;
		touch-action: none;
	}

	.region-preview .region-rect.dragging {
		cursor: grabbing;
	}

	.region-actions {
		display: flex;
		gap: 0.5rem;
		justify-content: center;
		flex-wrap: wrap;
	}

	.region-caption {
		font-size: 0.7rem;
		color: #888;
		text-align: center;
	}

	.preview-refresh {
		background: #2a2a2a;
		border: 1px solid #3a3a3a;
		border-radius: 6px;
		padding: 0.35rem 0.8rem;
		color: #ffffff;
		cursor: pointer;
		font-size: 0.7rem;
		font-family: 'JetBrains Mono', monospace;
	}

	.preview-refresh:hover:not(:disabled) {
		background: #333;
	}

	.preview-refresh:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}
</style>
