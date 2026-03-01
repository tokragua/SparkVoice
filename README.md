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

### 2. GPU Acceleration (NVIDIA CUDA)
To enable high-speed transcription on NVIDIA GPUs (RTX 20/30/40/50 series):

1. **[CUDA Toolkit 12.x](https://developer.nvidia.com/cuda-downloads)** — Download and install the latest CUDA Toolkit for Windows. This includes **cuBLAS**, the GPU math library that whisper.cpp uses internally.
   - During installation, ensure **"CUDA > Development > Libraries"** and **"CUDA > Runtime"** are checked.
   - The installer will set the `CUDA_PATH` environment variable automatically.

2. **[CMake](https://cmake.org/download/)** — Required to compile the native whisper.cpp library. Make sure it's added to your System PATH during installation.

3. **[LLVM/Clang](https://github.com/llvm/llvm-project/releases)** — Required for Rust-to-C++ FFI bindings (`bindgen`). Download the latest LLVM release for Windows, install it, and add the `bin` folder to your System PATH. Then set:
   ```powershell
   # Add to your PowerShell profile or run before building:
   $env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
   ```

> [!IMPORTANT]
> **End-user requirement**: Users running a CUDA-enabled SparkVoice build must have the **NVIDIA CUDA Runtime** installed on their system. The CUDA Toolkit is only needed for *compiling* — for running the app, the CUDA runtime (included with recent NVIDIA GPU drivers) is sufficient.

---

## 🏗️ How to Compile

### 1. Clone the Repository
```powershell
git clone https://github.com/tokragua/SparkVoice.git
cd SparkVoice
```

### 2. Install Dependencies
```powershell
npm install
```

### 3. Run in Development Mode
To start the app with local debugging:

```powershell
# 1. Standard Mode (Highly Portable / CPU-only)
# Best for development on all hardware (Intel/AMD/NVIDIA).
npm run tauri dev

# 2. Performance Mode (NVIDIA GPU / CUDA)
# Best for testing GPU acceleration. Requires CUDA Toolkit.
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
npm run tauri dev -- --features cuda
```

---

## 📦 Creating a Windows Executable (.exe / .msi)

To generate a production-ready installer:

### Standard Build (CPU-Only / Default)
This build is highly portable and works on any Windows machine regardless of GPU.
```powershell
npm run tauri build
```

### High-Performance Build (GPU/CUDA)
This build enables hardware acceleration but requires the user to have an NVIDIA GPU and the CUDA Toolkit installed for compilation.
```powershell
# Set LLVM Path for Bindings
$env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"

# Build with CUDA Feature
npm run tauri build -- --features cuda
```

> [!TIP]
> **Troubleshooting CUDA Builds**: If you see "Unresolved Externals" linker errors, ensure your `CUDA_PATH` environment variable is set correctly and that `LLVM` is installed and in your `PATH`.

The output will be located in: `src-tauri/target/release/bundle/msi/`

---

## ⚙️ Configuration

- **Hotkey**: Default is `F2`. Press once to start recording, press again to transcribe.
- **Models**: The first time you use a model, the app will download it automatically from HuggingFace.
- **Device**: Switch between "CPU" and "CUDA" (GPU) in the Settings panel to optimize for your hardware.

---

## 📝 Roadmap

- [ ] Add support for macOS

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

---

## 🙏 Credits

Built with ❤️ using:
- [Tauri](https://tauri.app/)
- [whisper-rs](https://github.com/tazz4843/whisper-rs)
- [whisper.cpp](https://github.com/ggerganov/whisper.cpp)
