<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { getCurrentWindow, primaryMonitor } from '@tauri-apps/api/window';
	import { open } from '@tauri-apps/plugin-dialog';
	import { openPath, revealItemInDir } from '@tauri-apps/plugin-opener';
	import { register, unregister } from '@tauri-apps/plugin-global-shortcut';
	import { onMount, onDestroy } from 'svelte';
	import {
		MonitorDot,
		Sun,
		Moon,
		Folder,
		FolderOpen,
		Play,
		Square,
		Circle,
		Maximize2,
		Crosshair,
		RefreshCw,
		ChevronRight,
	} from '@lucide/svelte';

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

	function parsePreset(key: string): { w: number; h: number } | null {
		const m = key.match(/^(\d+)x(\d+)$/);
		if (!m) return null;
		return { w: Number(m[1]), h: Number(m[2]) };
	}

	let theme = $state<'dark' | 'light'>('dark');
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
	const previewBox = $derived.by(() => {
		if (!screenSize) return null;
		const maxW = 280;
		const maxH = 150;
		const scale = Math.min(maxW / screenSize.w, maxH / screenSize.h);
		return { w: screenSize.w * scale, h: screenSize.h * scale, scale };
	});

	function currentRegionSize(): { w: number; h: number } | null {
		if (!screenSize) return null;
		const preset = parsePreset(regionPreset);
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
		const pos = clampPosition(
			(screenSize!.w - size.w) / 2,
			(screenSize!.h - size.h) / 2,
			size.w,
			size.h,
		);
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

	$effect(() => {
		const size = currentRegionSize();
		if (!size) return;
		const pos = clampPosition(regionX, regionY, size.w, size.h);
		if (pos.x !== regionX) regionX = pos.x;
		if (pos.y !== regionY) regionY = pos.y;
	});

	$effect(() => {
		document.documentElement.dataset.theme = theme;
		try {
			localStorage.setItem('theme', theme);
		} catch {
			// ignore
		}
	});

	function toggleTheme() {
		theme = theme === 'dark' ? 'light' : 'dark';
	}

	onMount(async () => {
		try {
			const saved = localStorage.getItem('theme');
			if (saved === 'dark' || saved === 'light') theme = saved;
		} catch {
			// ignore
		}

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

		const monitor = await primaryMonitor();
		if (monitor) {
			screenSize = { w: monitor.size.width, h: monitor.size.height };
			centerRegion();
		}

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
			// ignore
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

		// Close the layout overlay first and give the compositor a beat to
		// unmap it so the red outline never lands in the first frame.
		if (overlayVisible) {
			try {
				await invoke('hide_region_overlay');
			} catch {
				// ignore
			}
			overlayVisible = false;
			await new Promise((r) => setTimeout(r, 150));
		}

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
		if (!sources.find((s) => s.id === selectedSource)) {
			const screen = sources.find((s) => s.source_type === 'screen');
			selectedSource = screen ? screen.id : (sources[0]?.id ?? '');
		}
	}

	async function pickOutputDir() {
		const selected = await open({ directory: true, multiple: false });
		if (selected) outputDir = selected as string;
	}

	async function openVideo() {
		try {
			await openPath(outputPath);
		} catch (e) {
			alert(`Failed to open video: ${e}`);
		}
	}

	let dragStart = $state<{
		mouseX: number;
		mouseY: number;
		regionX: number;
		regionY: number;
	} | null>(null);

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
		const pos = clampPosition(
			dragStart.regionX + dxScreen,
			dragStart.regionY + dyScreen,
			size.w,
			size.h,
		);
		regionX = pos.x;
		regionY = pos.y;
	}

	function onRectPointerUp(e: PointerEvent) {
		const target = e.currentTarget as SVGElement;
		if (target.hasPointerCapture(e.pointerId)) target.releasePointerCapture(e.pointerId);
		dragStart = null;
	}

	function onRectKeyDown(e: KeyboardEvent) {
		if (recording) return;
		const step = e.shiftKey ? 50 : 10;
		let dx = 0;
		let dy = 0;
		switch (e.key) {
			case 'ArrowLeft':
				dx = -step;
				break;
			case 'ArrowRight':
				dx = step;
				break;
			case 'ArrowUp':
				dy = -step;
				break;
			case 'ArrowDown':
				dy = step;
				break;
			default:
				return;
		}
		e.preventDefault();
		const size = currentRegionSize();
		if (!size) return;
		const pos = clampPosition(regionX + dx, regionY + dy, size.w, size.h);
		regionX = pos.x;
		regionY = pos.y;
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

	$effect(() => {
		if (!overlayVisible) return;
		// Skip updates mid-drag; onRectPointerUp fires one final sync.
		if (dragStart) return;
		if (!previewRegion) return;
		invoke('show_region_overlay', previewRegion).catch(() => {});
	});

	$effect(() => {
		if (!canUseRegion && overlayVisible) {
			invoke('hide_region_overlay').catch(() => {});
			overlayVisible = false;
		}
	});

	async function showInFolder() {
		try {
			await revealItemInDir(outputPath);
		} catch (e) {
			alert(`Failed to open folder: ${e}`);
		}
	}
</script>

<main>
	<header class="topbar">
		<div class="brand">
			<MonitorDot size={20} strokeWidth={1.75} />
			<span>slop screen recorder</span>
		</div>
		<button class="icon-btn" onclick={toggleTheme} title="Toggle theme" aria-label="Toggle theme">
			{#if theme === 'dark'}
				<Sun size={14} strokeWidth={1.5} />
			{:else}
				<Moon size={14} strokeWidth={1.5} />
			{/if}
		</button>
	</header>

	{#if loading}
		<!-- init -->
	{:else if !ffmpegAvailable}
		<div class="error">
			<p>FFmpeg not found.</p>
			<p class="hint">Install FFmpeg and make sure it's available in your PATH.</p>
		</div>
	{:else}
		<div class="scroll-area">
			<div class="controls">
				<div class="field">
					<label for="source">Source</label>
					<select
						id="source"
						bind:value={selectedSource}
						disabled={recording}
						onfocus={refreshSources}
					>
						{#each sources as source (source.id)}
							<option value={source.id}>{source.name}</option>
						{/each}
					</select>
				</div>

				{#if canUseRegion}
					<label class="check">
						<input type="checkbox" bind:checked={useRegion} disabled={recording} />
						<span>Record region only</span>
					</label>

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
										role="button"
										tabindex="0"
										aria-label="Region position — drag or use arrow keys (hold shift for larger steps)"
										onpointerdown={onRectPointerDown}
										onpointermove={onRectPointerMove}
										onpointerup={onRectPointerUp}
										onpointercancel={onRectPointerUp}
										onkeydown={onRectKeyDown}
									/>
								</svg>
								<div class="region-caption">
									{screenSize.w}×{screenSize.h} · region {previewRegion.width}×{previewRegion.height}
									@ {previewRegion.x},{previewRegion.y}
								</div>
								<div class="region-actions">
									<button class="ghost" onclick={centerRegion} disabled={recording}>
										<Crosshair size={13} strokeWidth={1.5} /> Center
									</button>
									<button class="ghost" onclick={toggleOverlay} disabled={recording}>
										{#if overlayVisible}
											<RefreshCw size={13} strokeWidth={1.5} /> Hide overlay
										{:else}
											<Maximize2 size={13} strokeWidth={1.5} /> Show overlay
										{/if}
									</button>
								</div>
							</div>
						{/if}
					{/if}
				{/if}

				<details class="settings">
					<summary>
						<span class="chev"><ChevronRight size={14} strokeWidth={1.5} /></span>
						<span>Settings</span>
					</summary>
					<div class="settings-body">
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

						<div class="checks">
							<label class="check">
								<input type="checkbox" bind:checked={captureAudio} disabled={recording} />
								<span>Capture audio</span>
							</label>
							<label class="check">
								<input type="checkbox" bind:checked={minimizeOnRecord} disabled={recording} />
								<span>Minimize on record</span>
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
								<button
									class="ghost"
									onclick={pickOutputDir}
									disabled={recording}
									aria-label="Browse"
								>
									<Folder size={14} strokeWidth={1.5} />
								</button>
							</div>
						</div>
					</div>
				</details>
			</div>
		</div>

		<div class="record-bar">
			{#if outputPath && !recording}
				<div class="output-info">
					<code title={outputPath}>{outputPath.split(/[/\\]/).pop()}</code>
					<div class="output-actions">
						<button class="ghost" onclick={openVideo} aria-label="Open video">
							<Play size={13} strokeWidth={1.5} />
						</button>
						<button class="ghost" onclick={showInFolder} aria-label="Show in folder">
							<FolderOpen size={13} strokeWidth={1.5} />
						</button>
					</div>
				</div>
			{/if}

			<div class="record-section">
				{#if recording}
					<div class="timer">
						<span class="rec-pulse"></span>
						{formatTime(elapsed)}
					</div>
					<button class="btn" onclick={stopRecording}>
						<Square size={14} strokeWidth={1.5} fill="currentColor" /> Stop
					</button>
				{:else}
					<button class="btn primary" onclick={startRecording} disabled={!selectedSource}>
						<Circle size={14} strokeWidth={1.5} /> Record
					</button>
				{/if}
			</div>
		</div>
	{/if}
</main>

<style>
	:global(:root),
	:global([data-theme='dark']) {
		--bg: #0d0d0d;
		--surface: #151515;
		--surface-2: #1e1e1e;
		--border: #262626;
		--border-strong: #3a3a3a;
		--text: #e6e6e6;
		--text-dim: #888;
		--text-faint: #555;
		--accent: #e6e6e6;
		--rec: #ef4444;
	}

	:global([data-theme='light']) {
		--bg: #fafafa;
		--surface: #ffffff;
		--surface-2: #f2f2f2;
		--border: #e5e5e5;
		--border-strong: #c8c8c8;
		--text: #111111;
		--text-dim: #6b6b6b;
		--text-faint: #aaaaaa;
		--accent: #111111;
		--rec: #dc2626;
	}

	:global(html),
	:global(body) {
		margin: 0;
		background: var(--bg);
		color: var(--text);
		height: 100%;
	}

	:global(body) {
		font-family: 'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, monospace;
		font-size: 13px;
		line-height: 1.4;
		overflow: hidden;
	}

	main {
		max-width: 440px;
		margin: 0 auto;
		height: 100vh;
		display: flex;
		flex-direction: column;
		padding: 0 1rem;
		box-sizing: border-box;
	}

	.topbar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 0.75rem 0;
		border-bottom: 1px solid var(--border);
		flex-shrink: 0;
	}

	.scroll-area {
		flex: 1;
		overflow-y: auto;
		padding: 0.75rem 0;
		min-height: 0;
	}

	.record-bar {
		flex-shrink: 0;
		padding: 0.75rem 0 1rem;
		border-top: 1px solid var(--border);
		background: var(--bg);
		display: flex;
		flex-direction: column;
		gap: 0.6rem;
	}

	.settings > summary {
		display: inline-flex;
		align-items: center;
		gap: 0.3rem;
		cursor: pointer;
		list-style: none;
		font-size: 0.72rem;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--text-dim);
		user-select: none;
		padding: 0.2rem 0;
	}

	.settings > summary::-webkit-details-marker {
		display: none;
	}

	.settings > summary:hover {
		color: var(--text);
	}

	.settings .chev {
		display: inline-flex;
		transition: transform 0.15s;
	}

	.settings[open] .chev {
		transform: rotate(90deg);
	}

	.settings-body {
		display: flex;
		flex-direction: column;
		gap: 0.55rem;
		margin-top: 0.5rem;
	}

	.brand {
		display: inline-flex;
		align-items: center;
		gap: 0.55rem;
		font-size: 1rem;
		font-weight: 600;
		letter-spacing: -0.01em;
		text-transform: lowercase;
		color: var(--text);
	}

	.error {
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: 2px;
		padding: 1rem;
		text-align: center;
	}

	.error p {
		margin: 0.25rem 0;
	}

	.hint {
		font-size: 0.75rem;
		color: var(--text-dim);
	}

	code {
		background: var(--surface-2);
		padding: 0.15rem 0.35rem;
		border-radius: 2px;
		font-size: 0.72rem;
		font-family: inherit;
		color: var(--text);
	}

	.controls {
		display: flex;
		flex-direction: column;
		gap: 0.55rem;
	}

	.field {
		display: flex;
		flex-direction: column;
		gap: 0.2rem;
	}

	label {
		font-size: 0.68rem;
		color: var(--text-dim);
		text-transform: uppercase;
		letter-spacing: 0.06em;
	}

	.check {
		display: inline-flex;
		align-items: center;
		gap: 0.45rem;
		color: var(--text);
		text-transform: none;
		letter-spacing: normal;
		font-size: 0.78rem;
		cursor: pointer;
	}

	.checks {
		display: flex;
		gap: 1rem;
		flex-wrap: wrap;
	}

	select,
	input[type='text'] {
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: 2px;
		padding: 0.35rem 0.5rem;
		color: var(--text);
		font-size: 0.78rem;
		font-family: inherit;
	}

	select:focus,
	input:focus {
		outline: none;
		border-color: var(--border-strong);
	}

	.dir-picker {
		display: flex;
		gap: 0.35rem;
	}

	.dir-picker input {
		flex: 1;
	}

	button {
		font-family: inherit;
	}

	.ghost {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		gap: 0.35rem;
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: 2px;
		padding: 0.3rem 0.6rem;
		color: var(--text);
		cursor: pointer;
		font-size: 0.72rem;
	}

	.ghost:hover:not(:disabled) {
		border-color: var(--border-strong);
	}

	.ghost:disabled {
		opacity: 0.4;
		cursor: not-allowed;
	}

	.icon-btn {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 26px;
		height: 26px;
		background: transparent;
		border: 1px solid var(--border);
		border-radius: 2px;
		color: var(--text);
		cursor: pointer;
	}

	.icon-btn:hover {
		border-color: var(--border-strong);
	}

	.timer {
		display: inline-flex;
		align-items: center;
		gap: 0.45rem;
		font-size: 1.4rem;
		font-weight: 600;
		font-variant-numeric: tabular-nums;
		color: var(--text);
	}

	.rec-pulse {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--rec);
		animation: pulse 1s ease-in-out infinite;
	}

	@keyframes pulse {
		0%,
		100% {
			opacity: 1;
		}
		50% {
			opacity: 0.3;
		}
	}

	.btn {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		gap: 0.4rem;
		border: 1px solid var(--border-strong);
		background: var(--surface);
		color: var(--text);
		border-radius: 2px;
		padding: 0.5rem 1.25rem;
		font-size: 0.8rem;
		cursor: pointer;
		transition: background 0.1s;
	}

	.btn:disabled {
		opacity: 0.35;
		cursor: not-allowed;
	}

	.btn:hover:not(:disabled) {
		border-color: var(--text-dim);
	}

	.btn.primary {
		background: var(--accent);
		color: var(--bg);
		border-color: var(--accent);
	}

	.btn.primary:hover:not(:disabled) {
		opacity: 0.85;
	}

	.output-info {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: 2px;
		padding: 0.35rem 0.5rem;
		font-size: 0.7rem;
	}

	.output-info code {
		flex: 1;
		min-width: 0;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
		background: transparent;
		padding: 0;
	}

	.output-actions {
		display: flex;
		gap: 0.3rem;
		flex-shrink: 0;
	}

	.output-actions .ghost {
		padding: 0.25rem 0.4rem;
	}

	.record-section {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 0.45rem;
		margin-top: 0;
	}

	input[type='checkbox'] {
		accent-color: var(--accent);
		width: 13px;
		height: 13px;
	}

	.region-preview {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 0.4rem;
		padding: 0.55rem;
		background: var(--surface);
		border: 1px solid var(--border);
		border-radius: 2px;
	}

	.region-preview svg {
		display: block;
	}

	.region-preview .screen-rect {
		fill: var(--surface-2);
		stroke: var(--border-strong);
		stroke-width: 1;
	}

	.region-preview .region-rect {
		fill: var(--text);
		fill-opacity: 0.15;
		stroke: var(--text);
		stroke-width: 1;
		cursor: grab;
		touch-action: none;
	}

	.region-preview .region-rect.dragging {
		cursor: grabbing;
	}

	.region-preview .region-rect:focus {
		outline: none;
		stroke-width: 2;
		fill-opacity: 0.25;
	}

	.region-actions {
		display: flex;
		gap: 0.4rem;
		justify-content: center;
		flex-wrap: wrap;
	}

	.region-caption {
		font-size: 0.68rem;
		color: var(--text-dim);
		text-align: center;
	}
</style>
