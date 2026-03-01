import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const pillContainer = document.getElementById("pill-container") as HTMLElement;
const micToggle = document.getElementById("mic-toggle") as HTMLElement;
const canvas = document.getElementById("waveform") as HTMLCanvasElement;
const ctx = canvas.getContext("2d")!;

let isRecording = false;
let isTranscribing = false;
let lastTranscription = "";
let currentAmplitude = 0;
let smoothedAmplitude = 0;
let sweepPos = 0;
let sweepDirection = 1;
let isCompact = false;

function resizeCanvas() {
    canvas.width = canvas.clientWidth * window.devicePixelRatio;
    canvas.height = canvas.clientHeight * window.devicePixelRatio;
}

window.addEventListener("resize", resizeCanvas);
// Call resize on load and also handle if it's 0 early on
window.addEventListener("DOMContentLoaded", resizeCanvas);
resizeCanvas();

// Disable right-click context menu
window.addEventListener("contextmenu", (e) => e.preventDefault());

function drawWaveform() {
    if (!ctx) return;

    if (canvas.width === 0) {
        resizeCanvas();
    }

    ctx.clearRect(0, 0, canvas.width, canvas.height);

    const width = canvas.width;
    const height = canvas.height;
    const centerY = height / 2;

    if (isTranscribing) {
        // Loading "sweep" animation
        ctx.beginPath();
        ctx.strokeStyle = "#ffffff";
        ctx.lineWidth = 3;
        ctx.lineCap = "round";

        const bars = 20;
        const barWidth = width / bars;

        // Update sweep position
        sweepPos += 0.4 * sweepDirection;
        if (sweepPos >= bars || sweepPos <= 0) {
            sweepDirection *= -1;
        }

        for (let i = 0; i < bars; i++) {
            const x = i * barWidth + barWidth / 2;
            // Distance from sweep center (gaussian-ish)
            const dist = Math.abs(i - sweepPos);
            const intensity = Math.max(0, 1 - dist / 4);
            const amplitude = 5 + intensity * (height * 0.6);

            ctx.moveTo(x, centerY - amplitude / 2);
            ctx.lineTo(x, centerY + amplitude / 2);
        }
        ctx.stroke();
    } else {
        // Normal/Recording waveform
        // Smooth the amplitude: fast attack, slow decay
        if (currentAmplitude > smoothedAmplitude) {
            smoothedAmplitude = smoothedAmplitude * 0.2 + currentAmplitude * 0.8;
        } else {
            smoothedAmplitude = smoothedAmplitude * 0.8 + currentAmplitude * 0.2;
        }

        ctx.beginPath();
        ctx.strokeStyle = isRecording ? "#00ff88" : "#666";
        ctx.lineWidth = 2;
        ctx.lineCap = "round";

        const bars = 20;
        const barWidth = width / bars;
        const time = Date.now() * 0.01;

        for (let i = 0; i < bars; i++) {
            const x = i * barWidth + barWidth / 2;
            let amplitude = 2;

            if (isRecording) {
                // Significantly reduced sensitivity to prevent peaking
                const sensitivity = Math.sqrt(smoothedAmplitude);
                const jitter = Math.sin(time + i * 0.8) * 1.2;
                const boost = 1.8;
                amplitude = (sensitivity * height * boost) + jitter + 3;

                // Subtle spectral variation
                const spectral = Math.sin(i * 0.5 + time * 0.1) * 1.5;
                amplitude += spectral;

                amplitude = Math.max(5, Math.min(height * 0.8, amplitude));
            } else {
                amplitude = Math.sin(time * 0.2 + i * 0.2) * 2 + 3;
            }

            ctx.moveTo(x, centerY - amplitude / 2);
            ctx.lineTo(x, centerY + amplitude / 2);
        }
        ctx.stroke();
    }

    requestAnimationFrame(drawWaveform);
}

drawWaveform();

micToggle.addEventListener("click", () => {
    invoke("toggle_recording");
});

const cancelIcon = document.querySelector(".cancel-icon") as HTMLElement | null;
cancelIcon?.addEventListener("click", (e: Event) => {
    e.stopPropagation();
    invoke("cancel_transcription");
});

const settingsBtn = document.getElementById("settings-btn") as HTMLElement | null;
settingsBtn?.addEventListener("click", (e: Event) => {
    e.stopPropagation();
    invoke("open_settings");
});

const copyBtn = document.getElementById("copy-btn") as HTMLElement | null;
copyBtn?.addEventListener("click", (e: Event) => {
    e.stopPropagation();
    if (lastTranscription) {
        navigator.clipboard.writeText(lastTranscription).then(() => {
            // Visual feedback
            const originalIcon = copyBtn.innerText;
            copyBtn.innerText = "check";
            copyBtn.style.color = "#00ff88";
            setTimeout(() => {
                copyBtn.innerText = originalIcon;
                copyBtn.style.color = "";
            }, 2000);
        });
    }
});

const compactToggle = document.getElementById("compact-toggle") as HTMLElement | null;
compactToggle?.addEventListener("click", (e: Event) => {
    e.stopPropagation();
    isCompact = !isCompact;
    applyCompactState(isCompact);
    invoke("set_pill_collapsed", { collapsed: isCompact });
});

function applyCompactState(compact: boolean) {
    isCompact = compact;
    if (isCompact) {
        pillContainer.classList.add("compact");
        if (compactToggle) { // Check if compactToggle exists before accessing its properties
            compactToggle.innerText = "chevron_right";
            compactToggle.title = "Expand Pill";
        }
    } else {
        pillContainer.classList.remove("compact");
        if (compactToggle) { // Check if compactToggle exists before accessing its properties
            compactToggle.innerText = "chevron_left";
            compactToggle.title = "Toggle Compact Mode";
        }
    }
    // Canvas dimensions changed - call multiple times during and after the 300ms CSS transition
    setTimeout(resizeCanvas, 50);
    setTimeout(resizeCanvas, 150);
    setTimeout(resizeCanvas, 310);
}

// Load initial state
invoke("get_settings").then((settings: any) => {
    if (settings.pill_collapsed) {
        applyCompactState(true);
    }
});

pillContainer.addEventListener("mousedown", (e: MouseEvent) => {
    // Check if we're not clicking interactive elements
    const target = e.target as HTMLElement;
    if (!target.closest("#mic-toggle") &&
        !target.closest(".status-container") &&
        !target.closest(".pill-actions")) {
        invoke("start_dragging");
    }
});

console.log("SparkVoice Pill Ready");

listen("recording-toggled", (event) => {
    isRecording = event.payload as boolean;
    if (isRecording) {
        pillContainer.classList.add("recording");
    } else {
        pillContainer.classList.remove("recording");
    }
});

listen("transcribing-toggled", (event) => {
    isTranscribing = event.payload as boolean;
    if (isTranscribing) {
        pillContainer.classList.add("transcribing");
    } else {
        pillContainer.classList.remove("transcribing");
    }
});

listen("audio-amplitude", (event) => {
    currentAmplitude = event.payload as number;
});

listen("transcribed-text", (event) => {
    lastTranscription = event.payload as string;
    console.log("Captured transcription:", lastTranscription);
});
