import {createMixer} from "./audio.mjs";

class PlaybackProcessor extends AudioWorkletProcessor {
    constructor({processorOptions}) {
        super();
        const {asset, sources, sustain, duration, initial, reportProgress, trackMonitor} = processorOptions;
        this.mixer = createMixer(asset, sources, sampleRate, trackMonitor);
        this.frame = 0;
        this.length = Math.round(duration * sampleRate);
        this.sustain = sustain;
        this.scheduled = initial.map(event => ({...event, frame: Math.round(event.time * sampleRate)}));
        this.scheduledIndex = 0;
        this.events = [];
        this.reportProgress = reportProgress;
        this.nextProgress = 0;
        this.port.onmessage = ({data}) => {
            if (data.stop) {
                this.finish();
                return;
            }
            if (this.finished) return;

            if (data.trackMonitor) {
                this.mixer.setTrackMonitor(data.trackMonitor);
                return;
            }

            const event = {...data, frame: this.frame};
            this.events.push(event);
            this.port.postMessage({event: {...data, time: this.frame / sampleRate}});
        };
    }

    process(inputs, [[left, right]]) {
        if (this.finished) return false;

        const remaining = Math.min(left.length, this.length - this.frame);
        const first = this.scheduledIndex;
        const end = this.frame + remaining;
        while (this.scheduledIndex < this.scheduled.length && this.scheduled[this.scheduledIndex].frame < end) {
            this.scheduledIndex++;
        }
        const events = this.scheduled.slice(first, this.scheduledIndex);
        events.push(...this.events);
        if (this.events.length) events.sort((first, second) => first.frame - second.frame);
        this.mixer.mix(left.subarray(0, remaining), right.subarray(0, remaining), events, this.sustain);
        this.events = [];
        this.frame = end;
        if (this.reportProgress && this.frame >= this.nextProgress) {
            this.port.postMessage({position: this.frame / sampleRate});
            this.nextProgress = this.frame + Math.round(sampleRate / 30);
        }
        if (this.frame < this.length) return true;
        this.finish();
        return false;
    }

    finish() {
        if (this.finished) return;

        this.finished = true;
        this.port.postMessage({ended: true, duration: this.frame / sampleRate});
    }
}

registerProcessor("atk-audio", PlaybackProcessor);
