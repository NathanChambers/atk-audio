import {encodeWav, sampleRate} from "./audio.mjs";

const recordingSeconds = 30;

export function setupTuneCapture(stopPlayback) {
    const dialog = document.getElementById("tune-dialog");
    const source = document.getElementById("tune-source");
    const help = document.getElementById("tune-help");
    const microphoneInput = document.getElementById("tune-microphone");
    const input = document.getElementById("tune-input");
    const button = document.getElementById("tune-record");
    const status = document.getElementById("tune-status");
    const preview = document.getElementById("tune-preview");
    const download = document.getElementById("tune-download");
    const link = download.querySelector("a");
    let capture = null;
    let recordingUrl = null;

    async function refreshInputs() {
        if (!navigator.mediaDevices?.enumerateDevices) return;

        try {
            const devices = await navigator.mediaDevices.enumerateDevices();
            const selected = input.value;
            const microphones = devices.filter(device => device.kind === "audioinput" && device.deviceId && device.deviceId !== "default");
            const options = microphones.map((device, index) => new Option(device.label || "Microphone " + (index + 1), device.deviceId));
            input.replaceChildren(new Option("System default", ""), ...options);
            if (microphones.some(device => device.deviceId === selected)) input.value = selected;
        } catch {
            // The system default remains available if enumeration fails.
        }
    }

    function releaseStream(take) {
        clearTimeout(take.timer);
        take.stream?.getTracks().forEach(track => track.stop());
    }

    function resetCapture(message = "") {
        const take = capture;
        capture = null;
        if (take) {
            if (take.recorder?.state === "recording") take.recorder.stop();
            releaseStream(take);
        }

        button.disabled = false;
        source.disabled = false;
        input.disabled = false;
        button.textContent = recordingUrl ? "Record again" : "Start recording";
        status.textContent = message;
    }

    async function finishCapture(take, chunks) {
        releaseStream(take);
        if (capture !== take) return;

        const stopMessage = take.stopRequested ? take.stopMessage : "The browser stopped recording unexpectedly.";
        button.disabled = true;
        status.textContent = "Preparing recording…";
        try {
            const recorded = new Blob(chunks);
            if (recorded.size === 0) {
                resetCapture(stopMessage || "The recorder stopped before it received any audio.");
                return;
            }

            const buffer = await recorded.arrayBuffer();
            const context = new OfflineAudioContext(1, 1, sampleRate);
            const audio = await context.decodeAudioData(buffer);
            if (capture !== take) return;

            const frames = Math.min(audio.length, sampleRate * recordingSeconds);
            if (stopMessage && frames < sampleRate * 0.15) {
                resetCapture(stopMessage);
                return;
            }

            const channels = [];
            let strongest = null;
            let meanSquare = -1;
            for (let channel = 0; channel < audio.numberOfChannels; channel++) {
                const candidate = audio.getChannelData(channel).subarray(0, frames);
                channels.push(candidate);
                let energy = 0;
                for (const sample of candidate) energy += sample * sample;
                const channelMeanSquare = energy / frames;
                if (channelMeanSquare > meanSquare) {
                    strongest = candidate;
                    meanSquare = channelMeanSquare;
                }
            }

            const samples = take.systemAudio ? channels.slice(0, 2) : [strongest];
            const wav = encodeWav(samples);
            if (recordingUrl) URL.revokeObjectURL(recordingUrl);
            recordingUrl = URL.createObjectURL(new Blob([wav], {type: "audio/wav"}));
            preview.src = recordingUrl;
            link.href = recordingUrl;
            link.download = take.systemAudio ? "system-audio.wav" : "sung-tune.wav";
            download.hidden = false;
            const quietMessage = take.systemAudio
                ? "Very little sound was captured. Check that audio is playing on the shared source."
                : "Very little sound was captured. Select another microphone or check its input level, then try again.";
            const message = meanSquare < 0.003 ** 2 ? quietMessage : "Recording ready.";
            resetCapture(stopMessage ? stopMessage + " " + message : message);
        } catch (error) {
            if (capture === take) resetCapture(stopMessage || "Could not prepare the recording. Try another take.");
        }
    }

    function stopRecording(take, message = "") {
        if (capture !== take || take.stopRequested || !take.recorder) return;

        take.stopRequested = true;
        take.stopMessage = message;
        button.disabled = true;
        clearTimeout(take.timer);
        if (take.recorder.state === "recording") take.recorder.stop();
    }

    async function record() {
        if (capture) {
            stopRecording(capture);
            return;
        }

        const systemAudio = source.value === "system";
        const mediaDevices = navigator.mediaDevices;
        const supported = systemAudio ? mediaDevices?.getDisplayMedia : mediaDevices?.getUserMedia;
        if (!supported || typeof MediaRecorder === "undefined") {
            status.textContent = systemAudio
                ? "System audio recording is unavailable in this browser."
                : "Microphone recording is unavailable in this browser.";
            return;
        }

        if (!systemAudio) stopPlayback();
        preview.pause();
        const take = {systemAudio, stream: null, recorder: null, timer: null, stopRequested: false, stopMessage: ""};
        capture = take;
        button.disabled = true;
        source.disabled = true;
        input.disabled = true;
        status.textContent = systemAudio ? "Waiting for audio sharing…" : "Waiting for microphone access…";
        const constraints = {channelCount: 1, echoCancellation: false, noiseSuppression: false};
        if (input.value) constraints.deviceId = {exact: input.value};
        try {
            take.stream = systemAudio
                ? await mediaDevices.getDisplayMedia({video: {displaySurface: "monitor"}, audio: true, systemAudio: "include", windowAudio: "system"})
                : await mediaDevices.getUserMedia({audio: constraints});
            if (capture !== take) {
                releaseStream(take);
                return;
            }

            const audioTracks = take.stream.getAudioTracks();
            if (!audioTracks.length) {
                resetCapture(systemAudio
                    ? "No audio was shared. For desktop apps, choose Entire Screen and turn on Also share system audio."
                    : "The microphone returned no audio track. Choose another input and try again.");
                return;
            }

            if (audioTracks.every(track => track.readyState === "ended")) {
                resetCapture("The audio source disconnected before recording started.");
                return;
            }

            const audioStream = systemAudio ? new MediaStream(audioTracks) : take.stream;
            const recorder = new MediaRecorder(audioStream);
            take.recorder = recorder;
            const chunks = [];
            recorder.addEventListener("dataavailable", event => chunks.push(event.data));
            recorder.addEventListener("stop", () => finishCapture(take, chunks), {once: true});
            recorder.addEventListener("error", event => {
                if (capture !== take) return;

                const detail = event.error?.name || "unknown error";
                resetCapture("The browser recorder failed (" + detail + ").");
            }, {once: true});
            for (const track of audioTracks) {
                track.addEventListener("ended", () => {
                    if (audioTracks.every(audio => audio.readyState === "ended")) {
                        stopRecording(take, systemAudio ? "The shared audio stream disconnected." : "The microphone disconnected.");
                    }
                }, {once: true});
            }

            recorder.addEventListener("start", () => {
                if (capture !== take || take.stopRequested) return;

                const microphone = audioTracks[0].label;
                status.textContent = systemAudio ? "Recording shared audio…" : (microphone ? "Recording from " + microphone + "…" : "Recording…");
            }, {once: true});
            status.textContent = "Starting recorder…";
            recorder.start();
            take.timer = setTimeout(() => stopRecording(take), recordingSeconds * 1000);
            button.disabled = false;
            button.textContent = "Stop recording";
            if (!systemAudio) refreshInputs();
        } catch (error) {
            if (capture !== take) return;

            if (systemAudio) {
                const operation = take.stream ? "Could not start the audio recorder" : "Could not share system audio";
                resetCapture(operation + " (" + error.name + ").");
            } else {
                resetCapture("Could not access the microphone. Check browser permissions and the input device.");
                refreshInputs();
            }
        }
    }

    document.getElementById("tune-open").addEventListener("click", () => {
        dialog.showModal();
        refreshInputs();
    });
    button.addEventListener("click", record);
    source.addEventListener("change", () => {
        const systemAudio = source.value === "system";
        microphoneInput.hidden = systemAudio;
        help.textContent = systemAudio
            ? "For desktop apps, choose Entire Screen and enable system audio. For a browser tab, share the tab with audio enabled."
            : "Record up to 30 seconds.";
        status.textContent = "";
    });
    dialog.addEventListener("close", () => {
        preview.pause();
        resetCapture();
    });
    window.addEventListener("pagehide", () => {
        if (dialog.open) dialog.close();
        preview.pause();
        resetCapture();
    });
}
