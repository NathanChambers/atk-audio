import {decodeWav, maxSeconds, presetEvents, presetValue, timelineDuration} from "./audio.mjs";
import {setupTuneCapture} from "./tune.mjs";

const element = id => document.getElementById(id);
let selection = null;
let documentTag = null;
let projectTag = null;
let nextDiscovery = 0;
let refreshTimer = null;
let refreshController = null;
let refreshStopped = false;
let asset = null;
let sources = {};
let values = {};
let context = null;
let playback = null;
let workspaces = [];
let workspaceIndex = 0;
let playbackIndex = 0;
let selectedPreset = null;
let selectedClip = null;
let arrangementDuration = 1;
let visibleDuration = 1;
let selectionVersion = 0;
let playbackVersion = 0;
let sliders = new Map();
const trackMonitor = new Map();
let trackButtons = new Map();
const waveformClips = new WeakMap();
const waveformObserver = new ResizeObserver(entries => {
    for (const entry of entries) {
        const canvas = entry.target;
        const scale = window.devicePixelRatio;
        const width = Math.round(entry.contentRect.width * scale);
        const height = Math.round(entry.contentRect.height * scale);
        if (!width || !height) continue;
        if (canvas.width === width && canvas.height === height) continue;

        const {clip, source} = waveformClips.get(canvas);
        canvas.width = width;
        canvas.height = height;
        drawWaveform(canvas, clip, source);
    }
});

function show(message) {
    element("status").textContent = message;
}

function action(id, callback) {
    element(id).addEventListener("click", () => Promise.resolve().then(callback).catch(error => show(error.message)));
}

async function checkResponse(response) {
    if (!response.ok) throw new Error(await response.text());
    return response;
}

async function requestSnapshot(path, tag, signal) {
    const headers = tag ? {"If-None-Match": tag} : {};
    const response = await fetch(path, {headers, signal, cache: "no-store"});
    if (response.status === 304) return null;

    await checkResponse(response);
    return {data: await response.json(), tag: response.headers.get("ETag")};
}

function button(text, callback) {
    const result = document.createElement("button");
    result.type = "button";
    result.textContent = text;
    result.addEventListener("click", () => Promise.resolve().then(callback).catch(error => show(error.message)));
    return result;
}

function connection(message, state = "") {
    element("connection").textContent = message;
    element("connection-dot").className = state;
}

async function refreshWorkspaces(signal) {
    const snapshot = await requestSnapshot("/api/project", projectTag, signal);
    if (!snapshot) return;

    const project = snapshot.data;
    workspaces = [...project.assets.map(entry => ({
        path: "/api/assets/" + entry.id,
        label: entry.name + " · Asset · r" + entry.revision,
    })), ...project.sessions.map(entry => ({
        path: "/api/sessions/" + entry.id,
        label: entry.name + " · Session · " + entry.id,
    }))];
    projectTag = snapshot.tag;
    if (element("workspace-picker").open) drawWorkspaces();
}

function scheduleRefresh(delay) {
    clearTimeout(refreshTimer);
    if (!refreshStopped && !refreshController) refreshTimer = setTimeout(refresh, delay);
}

async function refresh() {
    const controller = new AbortController();
    refreshController = controller;
    const signal = controller.signal;
    const version = selectionVersion;
    const timeout = setTimeout(() => controller.abort(), 5000);
    try {
        if (Date.now() >= nextDiscovery) {
            await refreshWorkspaces(signal);
            nextDiscovery = Date.now() + 5000;
        }
        if (version !== selectionVersion || refreshStopped) return;

        let updatePending = false;
        if (selection) updatePending = await refreshSelection(signal);
        else {
            element("workspace-path").textContent = "Audio workspace";
            show("Choose a published asset or session to load.");
        }
        if (version !== selectionVersion || refreshStopped) return;

        connection(updatePending ? "Live · Update after playback" : "Live", "live");
    } catch (error) {
        if (version !== selectionVersion || refreshStopped) return;

        connection("Reconnecting", "offline");
        if (!documentTag) show(signal.aborted ? "Waiting for the audio server…" : error.message);
    } finally {
        clearTimeout(timeout);
        controller.abort();
        refreshController = null;
        const delay = document.hidden ? 2000 : 400;
        scheduleRefresh(version === selectionVersion ? delay : 0);
    }
}

function drawWorkspaces() {
    workspaceIndex = drawPicker("workspace", workspaces, workspaceIndex, selection, entry => load(entry));
}

function drawPicker(name, entries, index, current, choose) {
    const picker = element(name + "-picker");
    const search = element(name + "-search");
    const query = search.value.trim().toLowerCase();
    const matches = entries.filter(entry => entry.label.toLowerCase().includes(query) || entry.path.includes(query));
    const results = element(name + "-results");
    results.replaceChildren();
    const activeIndex = Math.max(0, Math.min(index, matches.length - 1));
    matches.forEach((entry, optionIndex) => {
        const item = button(entry.label, () => {
            picker.close();
            return choose(entry);
        });
        item.id = name + "-option-" + optionIndex;
        item.tabIndex = -1;
        item.setAttribute("role", "option");
        item.setAttribute("aria-selected", String(optionIndex === activeIndex));
        if (current === entry.path) item.setAttribute("aria-current", "true");
        results.append(item);
    });
    element(name + "-empty").hidden = matches.length > 0;
    const selected = results.children[activeIndex];
    if (selected) {
        search.setAttribute("aria-activedescendant", selected.id);
        if (picker.open) selected.scrollIntoView({block: "nearest"});
    } else {
        search.removeAttribute("aria-activedescendant");
    }
    return activeIndex;
}

function openWorkspaces() {
    const picker = element("workspace-picker");
    if (picker.open) return;

    element("workspace-search").value = "";
    workspaceIndex = workspaces.findIndex(entry => entry.path === selection);
    picker.showModal();
    drawWorkspaces();
    nextDiscovery = 0;
    scheduleRefresh(0);
}

function load({path, label}) {
    if (selection === path) {
        scheduleRefresh(0);
        return;
    }

    selectionVersion++;
    stop();
    selection = path;
    documentTag = null;
    asset = null;
    sources = {};
    selectedPreset = null;
    selectedClip = null;
    trackMonitor.clear();
    element("clip-panel").open = false;
    element("control-panel").open = false;
    element("editor").hidden = true;
    element("footer").hidden = true;
    element("workspace-path").textContent = label;
    show("Loading audio…");
    refreshController?.abort();
    scheduleRefresh(0);
}

async function refreshSelection(signal) {
    const path = selection;
    const version = selectionVersion;
    const snapshot = await requestSnapshot(path, documentTag, signal);
    if (!snapshot || version !== selectionVersion || refreshStopped) return false;
    if (playback) return true;

    const next = snapshot.data.asset;
    const previousSources = new Map((asset?.sources ?? []).map(source => [source.hash, sources[source.id]]));
    const loaded = await Promise.all(next.sources.map(async source => {
        const previous = previousSources.get(source.hash);
        if (previous) return [source.id, previous];

        const response = await checkResponse(await fetch("/api/sources/" + source.hash, {signal}));
        const bytes = new Uint8Array(await response.arrayBuffer());
        return [source.id, decodeWav(bytes)];
    }));
    if (version !== selectionVersion || refreshStopped) return false;
    if (playback) return true;

    stop();
    applySnapshot(next, loaded);
    documentTag = snapshot.tag;
    const label = snapshot.data.revision === undefined ? "Session · " + snapshot.data.id : "Asset · r" + snapshot.data.revision;
    element("workspace-path").textContent = next.name + " · " + label;
    return false;
}

function applySnapshot(next, loaded) {
    const trackIds = new Set(next.tracks.map(track => track.id));
    for (const id of trackMonitor.keys()) {
        if (!trackIds.has(id)) trackMonitor.delete(id);
    }
    const nextValues = {};
    for (const control of next.controls) {
        if (control.type !== "slider") continue;

        const previous = asset?.controls.find(entry => entry.id === control.id);
        const value = previous?.value === control.value ? values[control.id] : control.value;
        nextValues[control.id] = Math.max(control.min, Math.min(control.max, value));
    }
    asset = next;
    sources = Object.fromEntries(loaded);
    values = nextValues;
    const track = asset.tracks.find(entry => entry.clips.some(clip => clip.id === selectedClip));
    const clip = track?.clips.find(entry => entry.id === selectedClip);
    if (clip) showDetails(track, clip);
    else {
        selectedClip = null;
        element("details").textContent = "Select a clip on the timeline.";
        element("clip-panel").open = false;
    }
    drawControls();
    const preset = (asset.presets ?? []).find(entry => entry.id === selectedPreset?.id);
    updatePlaybackSelection(preset ?? null);
    if (element("playback-picker").open) drawPlaybackOptions();
    drawTimeline();
    element("editor").hidden = false;
    element("footer").hidden = false;
    show("");
}

function drawControls() {
    const container = element("controls");
    container.replaceChildren();
    sliders = new Map();
    element("control-panel").hidden = asset.controls.length === 0;
    for (const control of asset.controls) {
        if (control.type === "trigger") {
            container.append(button(control.name, () => {
                if (playback) playback.node.port.postMessage({control: control.id});
                else show("Start playback to use " + control.name + ".");
            }));
            continue;
        }

        const label = document.createElement("label");
        label.className = "control";
        const caption = document.createElement("span");
        const slider = document.createElement("input");
        slider.type = "range";
        slider.min = control.min;
        slider.max = control.max;
        slider.step = "any";
        slider.value = values[control.id];
        caption.textContent = control.name + " " + Number(values[control.id].toFixed(2));
        sliders.set(control.id, {slider, caption, name: control.name});
        slider.addEventListener("input", () => {
            const value = slider.valueAsNumber;
            values[control.id] = value;
            caption.textContent = control.name + " " + Number(value.toFixed(2));
            if (playback) playback.node.port.postMessage({control: control.id, value});
        });
        label.append(caption, slider);
        container.append(label);
    }
}

function updatePlaybackSelection(preset) {
    selectedPreset = preset;
    const name = preset?.name ?? "Arrangement";
    element("playback-name").textContent = name;
    element("playback-name").title = name;
    element("playback-open").disabled = !asset.presets?.length;
}

function drawPlaybackOptions() {
    const entries = [{path: "", label: "Arrangement", preset: null}];
    for (const preset of asset.presets ?? []) {
        entries.push({path: preset.id, label: preset.name + " · " + preset.seconds + " s", preset});
    }
    playbackIndex = drawPicker("playback", entries, playbackIndex, selectedPreset?.id ?? "", ({preset}) => {
        if (preset === selectedPreset) return;

        stop();
        updatePlaybackSelection(preset);
    });
}

function openPlaybackPicker() {
    const presets = asset.presets ?? [];
    playbackIndex = selectedPreset ? presets.indexOf(selectedPreset) + 1 : 0;
    element("playback-search").value = "";
    element("playback-picker").showModal();
    drawPlaybackOptions();
}

function showPresetPosition(preset, position) {
    for (const lane of preset.lanes) {
        const {slider, caption, name} = sliders.get(lane.control);
        const value = presetValue(lane.points, position);
        slider.value = value;
        caption.textContent = name + " " + Number(value.toFixed(2));
    }
}

function lockControlsForPreset(locked) {
    for (const control of element("controls").querySelectorAll("input, button")) control.disabled = locked;
    element("play").disabled = locked;
    if (locked) return;

    for (const [id, {slider, caption, name}] of sliders) {
        slider.value = values[id];
        caption.textContent = name + " " + Number(values[id].toFixed(2));
    }
}

function drawTimeline() {
    const rows = element("track-rows");
    const ruler = element("timeline").querySelector(".ruler");
    waveformObserver.disconnect();
    rows.replaceChildren();
    ruler.replaceChildren();
    trackButtons = new Map();
    element("reset-track-monitor").disabled = trackMonitor.size === 0;
    arrangementDuration = timelineDuration(asset) || 1;
    if (!asset.tracks.length) {
        rows.textContent = "This asset has no tracks yet.";
        showPlaybackPosition(0);
        return;
    }

    for (let index = 0; index <= 4; index++) {
        const tick = document.createElement("span");
        tick.style.left = (index * 25) + "%";
        ruler.append(tick);
    }

    for (const track of asset.tracks) {
        const row = document.createElement("div");
        const heading = document.createElement("div");
        const title = document.createElement("strong");
        const monitor = document.createElement("div");
        const lane = document.createElement("div");
        row.className = "track";
        heading.className = "track-heading";
        monitor.className = "track-monitor";
        lane.className = "track-lanes";
        title.textContent = track.name;
        const buttons = {};
        for (const [key, label] of [["mute", "Mute"], ["solo", "Solo"]]) {
            const control = button(label, () => toggleTrackMonitor(track, key));
            control.setAttribute("aria-label", label + " " + track.name);
            monitor.append(control);
            buttons[key] = control;
        }
        trackButtons.set(track.id, {track, ...buttons});
        updateTrackButtons(track.id);
        heading.append(title, monitor);
        row.append(heading, lane);
        rows.append(row);

        const ends = [];
        const sublanes = [];
        const sorted = [...track.clips].sort((first, second) => first.start - second.start);
        for (const clip of sorted.filter(clip => !clip.trigger)) {
            let line = ends.findIndex(end => end <= clip.start);
            if (line < 0) {
                line = ends.length;
                const sublane = document.createElement("div");
                sublane.className = "lane";
                sublanes.push(sublane);
                lane.append(sublane);
            }
            ends[line] = clip.start + clip.duration;

            const item = drawClip(track, clip);
            item.style.setProperty("--clip-width", 100 * clip.duration + "%");
            item.style.setProperty("--clip-start", 100 * clip.start + "%");
            sublanes[line].append(item);
        }
        const triggers = document.createElement("div");
        triggers.className = "triggers";
        for (const clip of sorted.filter(clip => clip.trigger)) {
            const item = drawClip(track, clip);
            item.classList.add("triggered");
            triggers.append(item);
        }
        if (triggers.childElementCount) lane.append(triggers);
    }

    const overlay = document.createElement("div");
    const playhead = document.createElement("div");
    overlay.className = "playhead-overlay";
    playhead.id = "playhead";
    playhead.hidden = true;
    overlay.setAttribute("aria-hidden", "true");
    overlay.append(playhead);
    rows.append(overlay);
    setTimelineDuration(arrangementDuration);
    showPlaybackPosition(0);
}

function setTimelineDuration(duration) {
    visibleDuration = duration;
    element("timeline").style.setProperty("--seconds", duration);
    const ticks = element("timeline").querySelectorAll(".ruler span");
    ticks.forEach((tick, index) => {
        tick.textContent = (duration * index / 4).toFixed(2) + " s";
    });
}

function showPlaybackPosition(position) {
    if (position > visibleDuration) {
        setTimelineDuration(Math.min(playback.duration, Math.max(position, visibleDuration * 2)));
    }
    const playhead = element("playhead");
    if (playhead) playhead.style.left = (100 * position / visibleDuration) + "%";
    element("playback-time").textContent = position.toFixed(2) + " s";
}

function updateTrackButtons(id) {
    const {track, mute, solo} = trackButtons.get(id);
    const overrides = trackMonitor.get(id);
    mute.setAttribute("aria-pressed", String(Boolean(overrides?.mute ?? track.mute)));
    solo.setAttribute("aria-pressed", String(Boolean(overrides?.solo ?? track.solo)));
}

function toggleTrackMonitor(track, key) {
    const overrides = {...trackMonitor.get(track.id)};
    const active = Boolean(overrides[key] ?? track[key]);
    const next = !active;
    if (next === Boolean(track[key])) delete overrides[key];
    else overrides[key] = next;

    if (Object.keys(overrides).length) trackMonitor.set(track.id, overrides);
    else trackMonitor.delete(track.id);

    updateTrackButtons(track.id);
    sendTrackMonitor();
}

function sendTrackMonitor() {
    element("reset-track-monitor").disabled = trackMonitor.size === 0;
    if (playback) playback.node.port.postMessage({trackMonitor: Object.fromEntries(trackMonitor)});
}

function drawClip(track, clip) {
    const item = button("", () => showDetails(track, clip));
    item.className = "clip";
    item.title = clip.name;
    const canvas = document.createElement("canvas");
    canvas.width = 0;
    canvas.height = 0;
    const label = document.createElement("span");
    label.textContent = clip.name + (clip.trigger ? " · Trigger" : clip.loop ? " · Loop" : "");
    item.append(canvas, label);
    waveformClips.set(canvas, {clip, source: sources[clip.source]});
    waveformObserver.observe(canvas, {box: "device-pixel-content-box"});
    return item;
}

function drawWaveform(canvas, clip, source) {
    const pen = canvas.getContext("2d");
    pen.strokeStyle = "#83ddc4";
    const channel = source.samples[0];
    const width = clip.duration * source.rate / canvas.width;
    const span = clip.length * source.rate;
    const center = canvas.height / 2;
    const amplitude = Math.max(0, center - 1);
    pen.beginPath();
    for (let column = 0; column < canvas.width; column++) {
        let min = 0;
        let max = 0;
        const start = column * width;
        const stride = Math.max(1, width / 24);
        for (let offset = 0; offset < width; offset += stride) {
            const sample = start + offset;
            if (!clip.loop && sample >= span) break;
            const index = Math.floor(clip.trim * source.rate + sample % span);
            const value = channel[index] ?? 0;
            min = Math.min(min, value);
            max = Math.max(max, value);
        }
        pen.moveTo(column + 0.5, center - max * amplitude);
        pen.lineTo(column + 0.5, center - min * amplitude);
    }
    pen.stroke();
}

function showDetails(track, clip) {
    selectedClip = clip.id;
    const source = asset.sources.find(source => source.id === clip.source);
    const mappings = asset.mappings.filter(mapping => mapping.clip === clip.id);
    const fields = {
        Clip: clip.name,
        Track: track.name,
        "Track low-pass": track.lowPassHz ? track.lowPassHz + " Hz" : null,
        "Track high-pass": track.highPassHz ? track.highPassHz + " Hz" : null,
        Source: clip.source,
        Start: clip.start + " s",
        Trim: clip.trim + " s",
        "Source length": clip.length + " s",
        Duration: clip.duration + " s",
        Gain: clip.gain,
        Pan: clip.pan,
        "Fade in": clip.fadeIn + " s",
        "Fade out": clip.fadeOut + " s",
        Loop: clip.loop ? "On" : "Off",
        Trigger: clip.trigger,
        Prompt: source.prompt,
        Origin: source.provenance.location,
        License: source.provenance.license,
        Attribution: source.provenance.attribution,
        Mappings: mappings.map(mapping => mapping.control + " → " + mapping.target + " (" + mapping.min + "–" + mapping.max + ")").join(", "),
    };
    const list = document.createElement("dl");
    for (const [name, value] of Object.entries(fields)) {
        if (value == null || value === "") continue;
        const term = document.createElement("dt");
        const description = document.createElement("dd");
        term.textContent = name;
        description.textContent = value;
        list.append(term, description);
    }
    element("details").replaceChildren(list);
}

function initialEvents() {
    return Object.entries(values).map(([control, value]) => ({control, value, time: 0}));
}

function stop() {
    playbackVersion++;
    if (!playback) return;

    const current = playback;
    playback = null;
    current.node.disconnect();
    current.node.port.close();
    if (current.preset) {
        lockControlsForPreset(false);
        show("");
    }
    element("playhead").hidden = true;
    setTimelineDuration(arrangementDuration);
    showPlaybackPosition(0);
    element("stop").disabled = true;
}

async function play(preset = null) {
    if (!asset.tracks.some(track => track.clips.length)) throw new Error("This asset has no clips.");
    stop();
    const version = playbackVersion;
    if (!context) context = new AudioContext({sampleRate: 48000});
    await context.resume();
    await context.audioWorklet.addModule("/playback.mjs");
    if (version !== playbackVersion) return;

    startPlayback(preset);
}

function startPlayback(preset) {
    const sustain = preset?.sustain ?? false;
    const duration = preset?.seconds ?? (timelineDuration(asset) || maxSeconds);
    const initial = preset ? presetEvents(preset) : initialEvents();
    const node = new AudioWorkletNode(context, "audio-workbench", {
        numberOfInputs: 0,
        numberOfOutputs: 1,
        outputChannelCount: [2],
        processorOptions: {
            asset, sources, sustain, duration, initial, reportProgress: true,
            trackMonitor: Object.fromEntries(trackMonitor),
        },
    });
    playback = {node, duration, preset};
    node.port.onmessage = ({data}) => {
        if (playback?.node !== node) return;
        if (data.position !== undefined) {
            showPlaybackPosition(data.position);
            if (preset) showPresetPosition(preset, data.position);
        }
        if (data.ended) {
            stop();
            if (element("loop").checked) {
                try {
                    startPlayback(preset);
                } catch (error) {
                    stop();
                    show(error.message);
                }
            }
        }
    };
    node.onprocessorerror = () => {
        if (playback?.node !== node) return;

        stop();
        show("Audio playback failed.");
    };
    node.connect(context.destination);
    if (preset) {
        lockControlsForPreset(true);
        for (const [id, {slider, caption, name}] of sliders) {
            const control = asset.controls.find(control => control.id === id);
            slider.value = control.value;
            caption.textContent = name + " " + control.value;
        }
        showPresetPosition(preset, 0);
    }
    element("stop").disabled = false;
    element("playhead").hidden = false;
    show("");
}

action("workspace-open", openWorkspaces);
setupTuneCapture(stop);
action("playback-open", openPlaybackPicker);
for (const id of ["clip-panel", "control-panel"]) {
    const panel = element(id);
    panel.addEventListener("toggle", () => {
        if (!panel.open) return;

        const other = id === "clip-panel" ? "control-panel" : "clip-panel";
        element(other).open = false;
    });
}
for (const name of ["workspace", "playback"]) {
    const search = element(name + "-search");
    const results = element(name + "-results");
    const draw = name === "workspace" ? drawWorkspaces : drawPlaybackOptions;
    search.addEventListener("input", () => {
        if (name === "workspace") workspaceIndex = 0;
        else playbackIndex = 0;
        draw();
    });
    search.addEventListener("keydown", event => {
        if (event.key === "Enter") {
            results.querySelector('[aria-selected="true"]')?.click();
        } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            const delta = event.key === "ArrowDown" ? 1 : -1;
            if (name === "workspace") workspaceIndex += delta;
            else playbackIndex += delta;
            draw();
        } else {
            return;
        }
        event.preventDefault();
    });
    element(name + "-picker").addEventListener("click", event => {
        if (event.target === event.currentTarget) event.currentTarget.close();
    });
}
document.addEventListener("keydown", event => {
    if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== "p") return;

    event.preventDefault();
    if (!element("playback-picker").open) openWorkspaces();
});
action("reset-track-monitor", () => {
    trackMonitor.clear();
    for (const id of trackButtons.keys()) updateTrackButtons(id);
    sendTrackMonitor();
});
action("play", () => play(selectedPreset));
action("stop", stop);
window.addEventListener("pagehide", () => {
    refreshStopped = true;
    clearTimeout(refreshTimer);
    refreshController?.abort();
});
window.addEventListener("pageshow", event => {
    if (!event.persisted) return;

    refreshStopped = false;
    nextDiscovery = 0;
    scheduleRefresh(0);
});
scheduleRefresh(0);
