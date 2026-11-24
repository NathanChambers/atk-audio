import {deflateSync} from "node:zlib";

const plotWidth = 800;
const plotHeight = 360;
const leftMargin = 70;
const topMargin = 20;
const imageWidth = 900;
const imageHeight = 448;
const palette = [[8, 10, 25], [44, 33, 105], [130, 44, 118], [215, 77, 74], [249, 161, 60], [253, 247, 171]];
const glyphs = {
    "0": [7, 5, 5, 5, 7], "1": [2, 6, 2, 2, 7], "2": [7, 1, 7, 4, 7],
    "3": [7, 1, 7, 1, 7], "4": [5, 5, 7, 1, 1], "5": [7, 4, 7, 1, 7],
    "6": [7, 4, 7, 5, 7], "7": [7, 1, 2, 2, 2], "8": [7, 5, 7, 5, 7],
    "9": [7, 5, 7, 1, 7], ".": [0, 0, 0, 0, 2], "-": [0, 0, 7, 0, 0],
    "H": [5, 5, 7, 5, 5], "Z": [7, 1, 2, 4, 7], "S": [7, 4, 7, 1, 7],
    "D": [6, 5, 5, 5, 6], "B": [6, 5, 6, 5, 6], "F": [7, 4, 6, 4, 4],
    "L": [4, 4, 4, 4, 7], "R": [6, 5, 6, 5, 5], "M": [5, 7, 7, 5, 5],
};

function setting(value, fallback, minimum, maximum, name) {
    const result = value ?? fallback;
    if (!Number.isFinite(result) || result < minimum || result > maximum) {
        throw new Error(name + " must be between " + minimum + " and " + maximum);
    }
    return result;
}

export function createSpectrogram(audio, options = {}) {
    const {rate, samples} = audio;
    const {start, end} = analysisRange(audio, options);

    const fftSize = analysisFftSize(options);

    const scale = options.scale ?? "log";
    if (!["log", "linear"].includes(scale)) throw new Error("scale must be log or linear");

    const minHz = setting(options.minHz, scale === "log" ? 30 : 0, scale === "log" ? 1 : 0, rate / 2, "minHz");
    const maxHz = setting(options.maxHz, Math.min(16000, rate / 2), minHz, rate / 2, "maxHz");
    if (maxHz <= minHz) throw new Error("maxHz must be greater than minHz");

    const floorDb = setting(options.floorDb, -85, -150, -1, "floorDb");
    const ceilingDb = setting(options.ceilingDb, 0, floorDb + 1, 48, "ceilingDb");
    const settings = {start, end, minHz, maxHz, scale, floorDb, ceilingDb, fftSize};
    const levels = measureSpectrum(samples, rate, settings);
    const pixels = drawImage(levels, settings);
    return {bytes: encodePng(pixels), width: imageWidth, height: imageHeight, sampleRate: rate, ...settings};
}

export function analysisRange(audio, options) {
    const start = setting(options.start, 0, 0, audio.duration, "start");
    const end = setting(options.end, audio.duration, start, audio.duration, "end");
    if (end <= start) throw new Error("end must be after start");

    const first = Math.min(audio.samples[0].length - 1, Math.round(start * audio.rate));
    const last = Math.max(first + 1, Math.round(end * audio.rate));
    return {start, end, first, last};
}

export function analysisFftSize(options) {
    const fftSize = options.fftSize ?? 4096;
    if (![512, 1024, 2048, 4096, 8192, 16384].includes(fftSize)) throw new Error("fftSize must be a power of two from 512 to 16384");
    return fftSize;
}

function frequencyAt(fraction, {scale, minHz, maxHz}) {
    if (scale === "linear") return minHz + fraction * (maxHz - minHz);
    return minHz * (maxHz / minHz) ** fraction;
}

function measureSpectrum(samples, rate, settings) {
    const {start, end, fftSize} = settings;
    const firstFrame = Math.round(start * rate);
    const endFrame = Math.max(firstFrame + 1, Math.round(end * rate));
    const frameSpan = endFrame - firstFrame;
    const powerSpectrum = createPowerSpectrum(samples, fftSize);
    const rowBins = [];
    for (let row = 0; row < plotHeight; row++) {
        const low = frequencyAt(row / plotHeight, settings) * fftSize / rate;
        const high = frequencyAt((row + 1) / plotHeight, settings) * fftSize / rate;
        const center = (low + high) / 2;
        rowBins.push({first: Math.ceil(low), last: Math.floor(high), center});
    }

    const levels = new Float32Array(plotWidth * plotHeight);
    const subwindows = Math.max(1, Math.ceil(frameSpan / plotWidth / (fftSize / 4)));
    for (let column = 0; column < plotWidth; column++) {
        for (let subwindow = 0; subwindow < subwindows; subwindow++) {
            const position = (column + (subwindow + 0.5) / subwindows) / plotWidth;
            const center = firstFrame + Math.min(frameSpan - 1, Math.floor(frameSpan * position));
            const powers = powerSpectrum(center);
            for (let row = 0; row < rowBins.length; row++) {
                const {first, last, center} = rowBins[row];
                const index = row * plotWidth + column;
                let power = 0;
                if (first <= last) {
                    for (let bin = first; bin <= last; bin++) power = Math.max(power, powers[bin]);
                } else {
                    const lower = Math.floor(center);
                    const fraction = center - lower;
                    power = powers[lower] * (1 - fraction) + powers[lower + 1] * fraction;
                }
                levels[index] = Math.max(levels[index], power);
            }
        }
    }

    return levels;
}

export function createPowerSpectrum(samples, fftSize, rms = false) {
    const window = new Float64Array(fftSize);
    const reversal = new Uint16Array(fftSize);
    const real = new Float64Array(fftSize);
    const imaginary = new Float64Array(fftSize);
    const powers = new Float64Array(fftSize / 2 + 1);
    const cosines = new Float64Array(fftSize / 2);
    const sines = new Float64Array(fftSize / 2);
    let windowSum = 0;
    let windowEnergy = 0;
    for (let index = 0; index < fftSize; index++) {
        window[index] = 0.5 - 0.5 * Math.cos(2 * Math.PI * index / fftSize);
        windowSum += window[index];
        windowEnergy += window[index] ** 2;
        reversal[index] = (reversal[index >> 1] >> 1) | ((index & 1) * (fftSize >> 1));
    }
    for (let index = 0; index < cosines.length; index++) {
        const angle = -2 * Math.PI * index / fftSize;
        cosines[index] = Math.cos(angle);
        sines[index] = Math.sin(angle);
    }

    const normalization = 1 / ((rms ? fftSize * windowEnergy : windowSum ** 2) * samples.length);
    return center => {
        const offset = center - fftSize / 2;
        powers.fill(0);
        for (const channel of samples) {
            for (let index = 0; index < fftSize; index++) {
                const sample = channel[offset + index] ?? 0;
                real[reversal[index]] = sample * window[index];
            }
            imaginary.fill(0);
            transform(real, imaginary, cosines, sines);
            for (let bin = 0; bin < powers.length; bin++) {
                const weight = bin === 0 || bin === powers.length - 1 ? 1 : rms ? 2 : 4;
                powers[bin] += weight * (real[bin] ** 2 + imaginary[bin] ** 2) * normalization;
            }
        }
        return powers;
    };
}

function transform(real, imaginary, cosines, sines) {
    for (let size = 2; size <= real.length; size *= 2) {
        const half = size / 2;
        const stride = real.length / size;
        for (let start = 0; start < real.length; start += size) {
            for (let offset = 0; offset < half; offset++) {
                const left = start + offset;
                const right = left + half;
                const angle = offset * stride;
                const rotatedReal = real[right] * cosines[angle] - imaginary[right] * sines[angle];
                const rotatedImaginary = real[right] * sines[angle] + imaginary[right] * cosines[angle];
                real[right] = real[left] - rotatedReal;
                imaginary[right] = imaginary[left] - rotatedImaginary;
                real[left] += rotatedReal;
                imaginary[left] += rotatedImaginary;
            }
        }
    }
}

function createCanvas() {
    const pixels = Buffer.alloc(imageWidth * imageHeight * 3, 18);
    const ink = [227, 234, 243];
    const pixel = (x, y, color) => {
        if (x < 0 || x >= imageWidth || y < 0 || y >= imageHeight) return;
        const index = (y * imageWidth + x) * 3;
        pixels[index] = color[0];
        pixels[index + 1] = color[1];
        pixels[index + 2] = color[2];
    };
    const text = (value, x, y) => {
        for (const character of value) {
            const glyph = glyphs[character] ?? [];
            for (let row = 0; row < glyph.length; row++) {
                for (let column = 0; column < 3; column++) {
                    if (!(glyph[row] & (4 >> column))) continue;
                    for (let dy = 0; dy < 2; dy++) {
                        for (let dx = 0; dx < 2; dx++) pixel(x + column * 2 + dx, y + row * 2 + dy, ink);
                    }
                }
            }
            x += 8;
        }
    };
    return {pixels, ink, pixel, text};
}

function drawTimeAxis({pixel, text, ink}, start, end) {
    for (let tick = 0; tick <= 8; tick++) {
        const x = leftMargin + Math.round(tick / 8 * (plotWidth - 1));
        const seconds = (start + (end - start) * tick / 8).toFixed(end - start < 0.1 ? 4 : 2);
        text(seconds, Math.min(imageWidth - seconds.length * 8 - 2, x - seconds.length * 4), topMargin + plotHeight + 10);
        for (let y = topMargin + plotHeight; y < topMargin + plotHeight + 5; y++) pixel(x, y, ink);
    }
    text("S", 470, 416);
}

export function createWaveform(audio, options) {
    const {start, end, first, last} = analysisRange(audio, options);
    const canvas = createCanvas();
    const {pixel, text, pixels} = canvas;
    const {samples, rate} = audio;
    const height = plotHeight / samples.length;
    for (let channel = 0; channel < samples.length; channel++) {
        const top = topMargin + channel * height;
        const bottom = top + height - 4;
        const zero = Math.round((top + bottom) / 2);
        const toY = value => Math.round(zero - Math.max(-1, Math.min(1, value)) * (bottom - top) / 2);
        const label = samples.length === 1 ? "M" : channel === 0 ? "L" : "R";
        text(label, 13, zero - 4);
        text("1", 42, top);
        text("0", 42, zero - 4);
        text("-1", 34, bottom - 9);
        for (let column = 0; column < plotWidth; column++) {
            const index = first + Math.floor((last - first) * column / plotWidth);
            const next = Math.max(index + 1, first + Math.floor((last - first) * (column + 1) / plotWidth));
            let minimum = Infinity;
            let maximum = -Infinity;
            for (let frame = index; frame < next; frame++) {
                const value = samples[channel][frame];
                minimum = Math.min(minimum, value);
                maximum = Math.max(maximum, value);
            }
            const x = leftMargin + column;
            pixel(x, zero, [65, 79, 94]);
            const color = minimum < -1 || maximum > 1 ? [255, 140, 83] : [112, 222, 194];
            for (let y = toY(maximum); y <= toY(minimum); y++) pixel(x, y, color);
        }
    }

    drawTimeAxis(canvas, start, end);
    return {bytes: encodePng(pixels), width: imageWidth, height: imageHeight, sampleRate: rate, channels: samples.length, start, end};
}

function drawImage(levels, settings) {
    const {start, end, maxHz, minHz, scale, floorDb, ceilingDb} = settings;
    const canvas = createCanvas();
    const {pixel, text, pixels, ink} = canvas;
    const colorAt = fraction => {
        const position = Math.max(0, Math.min(1, fraction)) * (palette.length - 1);
        const index = Math.min(palette.length - 2, Math.floor(position));
        return palette[index].map((value, channel) => Math.round(value + (palette[index + 1][channel] - value) * (position - index)));
    };

    for (let row = 0; row < plotHeight; row++) {
        const y = topMargin + plotHeight - 1 - row;
        for (let column = 0; column < plotWidth; column++) {
            const db = 10 * Math.log10(levels[row * plotWidth + column]);
            pixel(leftMargin + column, y, colorAt((db - floorDb) / (ceilingDb - floorDb)));
        }
    }

    text("HZ", 14, 3);
    for (let tick = 0; tick <= 6; tick++) {
        const fraction = tick / 6;
        const y = topMargin + Math.round((1 - fraction) * (plotHeight - 1));
        const frequency = Math.round(frequencyAt(fraction, {minHz, maxHz, scale}));
        text(String(frequency), 14, y - 4);
        for (let x = leftMargin - 5; x < leftMargin; x++) pixel(x, y, ink);
    }
    drawTimeAxis(canvas, start, end);
    for (let column = 0; column < 180; column++) {
        const color = colorAt(column / 179);
        for (let row = 0; row < 8; row++) pixel(leftMargin + 44 + column, 416 + row, color);
    }
    text(String(floorDb), leftMargin + 5, 415);
    text(ceilingDb + " DBFS", leftMargin + 230, 415);

    return pixels;
}

function encodePng(pixels) {
    const header = Buffer.alloc(13);
    header.writeUInt32BE(imageWidth, 0);
    header.writeUInt32BE(imageHeight, 4);
    header[8] = 8;
    header[9] = 2;
    const stride = imageWidth * 3;
    const scanlines = Buffer.alloc((stride + 1) * imageHeight);
    for (let row = 0; row < imageHeight; row++) {
        pixels.copy(scanlines, row * (stride + 1) + 1, row * stride, (row + 1) * stride);
    }

    const signature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
    return Buffer.concat([signature, pngChunk("IHDR", header), pngChunk("IDAT", deflateSync(scanlines)), pngChunk("IEND", Buffer.alloc(0))]);
}

function pngChunk(type, data) {
    const bytes = Buffer.alloc(data.length + 12);
    bytes.writeUInt32BE(data.length, 0);
    bytes.write(type, 4, "ascii");
    data.copy(bytes, 8);
    let crc = 0xffffffff;
    for (let index = 4; index < data.length + 8; index++) {
        crc ^= bytes[index];
        for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0);
    }
    bytes.writeUInt32BE((crc ^ 0xffffffff) >>> 0, data.length + 8);
    return bytes;
}
