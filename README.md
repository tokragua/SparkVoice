# ✨ SparkVoice

**Experience the future of universal dictation.**  
SparkVoice is a high-performance, privacy-focused desktop application that brings OpenAI's Whisper models directly to your fingertips. Dictate into *any* application with zero-latency, multi-language auto-detection, and full GPU acceleration.

![SparkVoice UI](src-tauri/icons/32x32.png) <!-- Replace with a real screenshot if available -->

---

## 🚀 Key Features

- **🏎️ Hardware Accelerated**: Leverages NVIDIA CUDA for near-instant transcription on modern GPUs.
- **✨ Intelligent Auto-Detect**: Automatically switches between your preferred languages (e.g., English, Romanian, German) while ignoring others.
- **📦 Model Management**: Download, select, and delete models (Tiny to Large-v3) directly within the app.
- **🌊 Responsive Waveform**: Real-time audio visualization with high-energy peak detection.
- **⌨️ Universal Injection**: Press `F2` to dictate anywhere—Word, Chrome, Slack, or your favorite IDE.
- **🛡️ 100% Private**: Everything runs locally on your machine. No cloud, no subscriptions, no tracking.

---

## 🛠️ Prerequisites

Before compiling SparkVoice, ensure you have the following installed on your Windows system:

### 1. Development Environment
- **[Rust](https://rustup.rs/)**: The core backend language.
- **[Node.js (LTS)](https://nodejs.org/)**: Required for the Tauri frontend.
- **[Visual Studio 2022](https://visualstudio.microsoft.com/vs/community/)**: Install with the "Desktop development with C++" workload.

### 2. GPU Acceleration (Recommended)
To enable high-speed transcription on NVIDIA cards (e.g., RTX 30/40 series):
- **[CUDA Toolkit 12.8+](https://developer.nvidia.com/cuda-downloads)**: Essential for hardware acceleration.
- **[Latest CMake](https://cmake.org/download/)**: Ensure it is added to your System PATH.
- **[LLVM](https://github.com/llvm/llvm-project/releases)**: Required for Rust-to-C++ bindings (add the `bin` folder to your PATH).

---

## 🏗️ How to Compile

### 1. Clone the Repository
```powershell
git clone https://github.com/your-username/SparkVoice.git
cd SparkVoice
```

### 2. Install Dependencies
```powershell
npm install
```

### 3. Run in Development Mode
To start the app with local debugging:
```powershell
# For CPU-only mode:
npm run tauri dev

# For GPU (CUDA) mode:
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
npm run tauri dev -- --features cuda
```

---

## 📦 Creating a Windows Executable (.exe / .msi)

To generate a production-ready installer:

### Standard Build (CPU)
```powershell
npm run tauri build
```

### High-Performance Build (GPU/CUDA)
*Note: This requires the CUDA Toolset files to be correctly placed in your MSVC BuildCustomizations folder.*
```powershell
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
cargo tauri build --features cuda
```

The output will be located in: `src-tauri/target/release/bundle/msi/`

---

## ⚙️ Configuration

- **Hotkey**: Default is `F2`. Hold to record, release to transcribe.
- **Models**: The first time you use a model, the app will download it automatically from HuggingFace.
- **Device**: Switch between "CPU" and "CUDA" (GPU) in the Settings panel to optimize for your hardware.

---

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

---

## 🙏 Credits

Built with ❤️ using:
- [Tauri](https://tauri.app/)
- [whisper-rs](https://github.com/tazz4843/whisper-rs)
- [whisper.cpp](https://github.com/ggerganov/whisper.cpp)
