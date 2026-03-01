import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** Mirrors the Rust AppSettings struct */
interface AppSettings {
  selected_language: string;
  languages: string[];
  device: string;
  input_device: string | null;
  pill_x: number;
  pill_y: number;
  selected_model: string;
  launch_on_startup: boolean;
  recording_shortcut: string;
  max_recording_seconds: number;
  pill_collapsed: boolean;
}

/** Mirrors the Rust ModelMetadata struct */
interface ModelMetadata {
  name: string;
  size: string;
  description: string;
}

const WHISPER_LANGUAGES = [
  "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it", "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur", "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si", "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo", "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw", "su"
];

const LANGUAGE_NAMES: Record<string, string> = {
  "en": "English", "zh": "Chinese", "de": "German", "es": "Spanish", "ru": "Russian", "ko": "Korean", "fr": "French", "ja": "Japanese", "pt": "Portuguese", "tr": "Turkish", "pl": "Polish", "ca": "Catalan", "nl": "Dutch", "ar": "Arabic", "sv": "Swedish", "it": "Italian", "id": "Indonesian", "hi": "Hindi", "fi": "Finnish", "vi": "Vietnamese", "he": "Hebrew", "uk": "Ukrainian", "el": "Greek", "ms": "Malay", "cs": "Czech", "ro": "Romanian", "da": "Danish", "hu": "Hungarian", "ta": "Tamil", "no": "Norwegian", "th": "Thai", "ur": "Urdu", "hr": "Croatian", "bg": "Bulgarian", "lt": "Lithuanian", "la": "Latin", "mi": "Maori", "ml": "Malayalam", "cy": "Welsh", "sk": "Slovak", "te": "Telugu", "fa": "Persian", "lv": "Latvian", "bn": "Bengali", "sr": "Serbian", "az": "Azerbaijani", "sl": "Slovenian", "kn": "Kannada", "et": "Estonian", "mk": "Macedonian", "br": "Breton", "eu": "Basque", "is": "Icelandic", "hy": "Armenian", "ne": "Nepali", "mn": "Mongolian", "bs": "Bosnian", "kk": "Kazakh", "sq": "Albanian", "sw": "Swahili", "gl": "Galician", "mr": "Marathi", "pa": "Punjabi", "si": "Sinhala", "km": "Khmer", "sn": "Shona", "yo": "Yoruba", "so": "Somali", "af": "Afrikaans", "oc": "Occitan", "ka": "Georgian", "be": "Belarusian", "tg": "Tajik", "sd": "Sindhi", "gu": "Gujarati", "am": "Amharic", "yi": "Yiddish", "lo": "Lao", "uz": "Uzbek", "fo": "Faroese", "ht": "Haitian Creole", "ps": "Pashto", "tk": "Turkmen", "nn": "Nynorsk", "mt": "Maltese", "sa": "Sanskrit", "lb": "Luxembourgish", "my": "Myanmar", "bo": "Tibetan", "tl": "Tagalog", "mg": "Malagasy", "as": "Assamese", "tt": "Tatar", "haw": "Hawaiian", "ln": "Lingala", "ha": "Hausa", "ba": "Bashkir", "jw": "Javanese", "su": "Sundanese"
};

window.addEventListener("DOMContentLoaded", async () => {
  const audioInputSelect = document.getElementById("audio-input-select") as HTMLSelectElement;
  const deviceSelect = document.getElementById("device-select") as HTMLSelectElement;
  const languageSelect = document.getElementById("language-select") as HTMLSelectElement;
  const availableLanguagesSelect = document.getElementById("available-languages-select") as HTMLSelectElement;
  const myLanguagesList = document.getElementById("my-languages-list") as HTMLUListElement;
  const addLanguageBtn = document.getElementById("add-language-btn") as HTMLButtonElement;
  const launchStartupToggle = document.getElementById("launch-startup-toggle") as HTMLInputElement;
  const maxRecordingInput = document.getElementById("max-recording-input") as HTMLInputElement;
  const statusText = document.getElementById("status-text") as HTMLElement;

  const modelList = document.getElementById("model-list") as HTMLElement;
  const modelStatus = document.getElementById("model-status") as HTMLElement;

  const hotkeyDisplay = document.getElementById("hotkey-display") as HTMLElement;
  const appVersionDisplay = document.getElementById("app-version") as HTMLElement;
  let isRecordingHotkey = false;

  const sidebarNav = document.getElementById("sidebar-nav") as HTMLElement;
  const sections = document.querySelectorAll(".content-section");

  // Sidebar Navigation Logic
  sidebarNav.addEventListener("click", (e) => {
    const target = e.target as HTMLElement;
    const navItem = target.closest(".nav-item") as HTMLElement;
    if (!navItem) return;

    e.preventDefault();
    const sectionId = navItem.dataset.section;

    // Update nav items
    document.querySelectorAll(".nav-item").forEach(item => {
      item.classList.remove("active");
    });
    navItem.classList.add("active");

    // Show/Hide sections
    sections.forEach(section => {
      if (section.id === `section-${sectionId}`) {
        section.classList.remove("hidden");
      } else {
        section.classList.add("hidden");
      }
    });
  });

  listen("status-update", (event: any) => {
    statusText.innerText = event.payload as string;
  });

  listen("model-download-status", (event: any) => {
    modelStatus.innerText = event.payload as string;
    if (event.payload === "ready") {
      refreshUI();
      setTimeout(() => { modelStatus.innerText = ""; }, 3000);
    }
  });

  // Load and populate settings
  async function refreshUI() {
    try {
      const settings = await invoke<AppSettings>("get_settings");
      const availableModels = await invoke<ModelMetadata[]>("get_available_models");
      const downloadedModels = await invoke<string[]>("get_downloaded_models");

      if (launchStartupToggle) {
        launchStartupToggle.checked = settings.launch_on_startup;
      }

      if (maxRecordingInput) {
        maxRecordingInput.value = settings.max_recording_seconds.toString();
      }

      if (hotkeyDisplay) {
        hotkeyDisplay.innerText = settings.recording_shortcut;
      }

      const version = await invoke<string>("get_app_version");
      if (appVersionDisplay) {
        appVersionDisplay.innerText = `v${version}`;
      }

      // Populate active language dropdown
      languageSelect.innerHTML = '<option value="auto">✨ Auto Detect</option>';
      settings.languages.forEach((lang: string) => {
        const option = document.createElement("option");
        option.value = lang;
        option.text = LANGUAGE_NAMES[lang] || lang;
        languageSelect.add(option);
      });
      languageSelect.value = settings.selected_language;

      // Populate "My Languages" list
      myLanguagesList.innerHTML = "";
      settings.languages.forEach((lang: string) => {
        const li = document.createElement("li");
        li.className = "language-item";

        const leftDiv = document.createElement("div");
        leftDiv.className = "flex items-center gap-3";

        const checkIcon = document.createElement("span");
        checkIcon.className = "material-symbols-outlined text-primary text-sm";
        checkIcon.textContent = "check_circle";
        leftDiv.appendChild(checkIcon);

        const nameSpan = document.createElement("span");
        nameSpan.className = "text-white font-medium";
        nameSpan.textContent = LANGUAGE_NAMES[lang] || lang;
        leftDiv.appendChild(nameSpan);

        const codeSpan = document.createElement("span");
        codeSpan.className = "text-[#9cbaa6] text-xs uppercase font-bold tracking-widest";
        codeSpan.textContent = lang;
        leftDiv.appendChild(codeSpan);

        li.appendChild(leftDiv);

        const removeBtn = document.createElement("span");
        removeBtn.className = "material-symbols-outlined remove-btn text-[20px]";
        removeBtn.dataset.lang = lang;
        removeBtn.textContent = "close";
        li.appendChild(removeBtn);

        myLanguagesList.appendChild(li);
      });

      // Populate models list
      modelList.innerHTML = "";
      availableModels.forEach((model: any) => {
        const isDownloaded = downloadedModels.includes(model.name);
        const isSelected = settings.selected_model === model.name;

        const modelDiv = document.createElement("div");
        modelDiv.className = `glass-panel rounded-2xl p-4 flex items-center justify-between group hover:border-primary/30 transition-all duration-300 model-item ${isSelected ? "selected" : ""}`;

        // Left: icon + info
        const leftDiv = document.createElement("div");
        leftDiv.className = "flex items-center gap-4";

        const iconWrapper = document.createElement("div");
        iconWrapper.className = `flex items-center justify-center rounded-xl bg-[#28392e] ${isSelected ? 'text-primary' : 'text-white'} shrink-0 size-10 shadow-inner`;
        const icon = document.createElement("span");
        icon.className = "material-symbols-outlined text-[20px]";
        icon.textContent = "description";
        iconWrapper.appendChild(icon);
        leftDiv.appendChild(iconWrapper);

        const infoDiv = document.createElement("div");
        infoDiv.className = "flex flex-col";

        const nameRow = document.createElement("div");
        nameRow.className = "flex items-center gap-2";
        const nameP = document.createElement("p");
        nameP.className = "text-white font-semibold group-hover:text-primary transition-colors";
        nameP.textContent = model.name;
        nameRow.appendChild(nameP);
        if (isSelected) {
          const pill = document.createElement("span");
          pill.className = "status-pill";
          pill.textContent = "Active";
          nameRow.appendChild(pill);
        }
        infoDiv.appendChild(nameRow);

        const descP = document.createElement("p");
        descP.className = "text-[#9cbaa6] text-xs";
        descP.textContent = `${model.size} • ${model.description}`;
        infoDiv.appendChild(descP);

        leftDiv.appendChild(infoDiv);
        modelDiv.appendChild(leftDiv);

        // Right: action buttons
        const btnDiv = document.createElement("div");
        btnDiv.className = "flex gap-2";

        if (!isDownloaded) {
          const dlBtn = document.createElement("button");
          dlBtn.className = "download-btn px-4 py-1.5 rounded-lg bg-primary text-background-dark text-xs font-bold hover:scale-[1.05] transition-all";
          dlBtn.dataset.model = model.name;
          dlBtn.textContent = "Download";
          btnDiv.appendChild(dlBtn);
        }
        if (isDownloaded && !isSelected) {
          const useBtn = document.createElement("button");
          useBtn.className = "use-btn px-4 py-1.5 rounded-lg border border-primary/30 text-primary text-xs font-bold hover:bg-primary/10 transition-all";
          useBtn.dataset.model = model.name;
          useBtn.textContent = "Select";
          btnDiv.appendChild(useBtn);
        }
        if (isDownloaded) {
          const delBtn = document.createElement("button");
          delBtn.className = "delete-btn px-4 py-1.5 rounded-lg border border-red-500/30 text-red-400 text-xs font-bold hover:bg-red-500/10 transition-all";
          delBtn.dataset.model = model.name;
          delBtn.textContent = "Delete";
          btnDiv.appendChild(delBtn);
        }

        modelDiv.appendChild(btnDiv);
        modelList.appendChild(modelDiv);
      });

      // Populate device and microphone
      // Populate device and compute device
      const isCudaSupported: boolean = await invoke("is_cuda_supported");
      const cudaOption = deviceSelect.querySelector('option[value="cuda"]');

      if (!isCudaSupported) {
        if (cudaOption) {
          cudaOption.remove();
        }
        // If current setting is cuda but not supported, fall back to cpu
        if (settings.device === "cuda") {
          settings.device = "cpu";
          await invoke("set_device", { device: "cpu" });
        }
      } else if (!cudaOption) {
        // Re-add if it was previously removed but now supported
        const option = document.createElement("option");
        option.value = "cuda";
        option.text = "CUDA GPU";
        deviceSelect.prepend(option);
      }

      deviceSelect.value = settings.device;
      const devices: string[] = await invoke("get_input_devices");
      audioInputSelect.innerHTML = '<option value="">Default System Microphone</option>';
      devices.forEach(device => {
        const option = document.createElement("option");
        option.value = device;
        option.text = device;
        audioInputSelect.add(option);
      });
      audioInputSelect.value = settings.input_device || "";

    } catch (error) {
      console.error("Failed to load settings:", error);
    }
  }

  // Populate the wide list of available languages
  WHISPER_LANGUAGES.sort((a, b) => (LANGUAGE_NAMES[a] || a).localeCompare(LANGUAGE_NAMES[b] || b))
    .forEach(lang => {
      const option = document.createElement("option");
      option.value = lang;
      option.text = LANGUAGE_NAMES[lang] || lang;
      availableLanguagesSelect.add(option);
    });

  await refreshUI();

  // Auto-download tiny model if none exist (clean install)
  const downloadedModels = await invoke<string[]>("get_downloaded_models");
  if (downloadedModels.length === 0) {
    await invoke("download_model", { model: "tiny" }).catch(() => { });
  }

  // Event Listeners
  languageSelect.addEventListener("change", async () => {
    try {
      await invoke("set_language", { lang: languageSelect.value });
      statusText.innerText = "Language updated";
    } catch (err) {
      statusText.innerText = `Error: ${err}`;
    }
  });

  launchStartupToggle.addEventListener("change", async () => {
    try {
      await invoke("set_launch_on_startup", { enabled: launchStartupToggle.checked });
    } catch (err) {
      statusText.innerText = `Error: ${err}`;
    }
  });

  maxRecordingInput.addEventListener("change", async () => {
    const value = parseInt(maxRecordingInput.value);
    if (!isNaN(value) && value >= 10 && value <= 3600) {
      try {
        await invoke("set_max_recording_duration", { duration: value });
        statusText.innerText = "Recording limit updated";
      } catch (err) {
        statusText.innerText = `Error: ${err}`;
      }
    } else {
      statusText.innerText = "Duration must be 10–3600 seconds";
    }
  });

  addLanguageBtn.addEventListener("click", async () => {
    const lang = availableLanguagesSelect.value;
    if (lang) {
      try {
        await invoke("add_language", { lang });
        await refreshUI();
        availableLanguagesSelect.value = "";
        statusText.innerText = "Language added";
      } catch (err) {
        statusText.innerText = `Error: ${err}`;
      }
    }
  });

  myLanguagesList.addEventListener("click", async (e) => {
    const target = e.target as HTMLElement;
    if (target.classList.contains("remove-btn")) {
      const lang = target.getAttribute("data-lang");
      if (lang) {
        try {
          await invoke("remove_language", { lang });
          await refreshUI();
          statusText.innerText = "Language removed";
        } catch (err) {
          statusText.innerText = `Error: ${err}`;
        }
      }
    }
  });

  modelList.addEventListener("click", async (e) => {
    const target = e.target as HTMLElement;
    if (target.tagName === "BUTTON") {
      const model = target.getAttribute("data-model");
      if (!model) return;

      try {
        if (target.classList.contains("download-btn")) {
          await invoke("download_model", { model });
        } else if (target.classList.contains("use-btn")) {
          await invoke("select_model", { model });
          await refreshUI();
          statusText.innerText = `Switched to ${model} model`;
        } else if (target.classList.contains("delete-btn")) {
          if (confirm(`Are you sure you want to delete the ${model} model?`)) {
            await invoke("delete_model", { model });
            await refreshUI();
            statusText.innerText = `${model} model deleted`;
          }
        }
      } catch (err) {
        statusText.innerText = `Error: ${err}`;
      }
    }
  });

  audioInputSelect.addEventListener("change", async () => {
    try {
      await invoke("set_input_device", { device: audioInputSelect.value });
      statusText.innerText = "Microphone updated";
    } catch (error) {
      statusText.innerText = "Error updating microphone";
    }
  });

  deviceSelect.addEventListener("change", async () => {
    try {
      await invoke("set_device", { device: deviceSelect.value });
      statusText.innerText = `Switched to ${deviceSelect.value.toUpperCase()}`;
    } catch (err) {
      statusText.innerText = `Error: ${err}`;
    }
  });

  // Hotkey Recording Logic
  hotkeyDisplay.addEventListener("click", () => {
    isRecordingHotkey = true;
    hotkeyDisplay.innerText = "...";
    hotkeyDisplay.classList.add("ring-2", "ring-primary", "animate-pulse");
  });

  window.addEventListener("keydown", async (e: KeyboardEvent) => {
    if (!isRecordingHotkey) return;

    // Prevent default browser behavior (e.g., F2 for rename)
    e.preventDefault();

    const modifiers: string[] = [];
    if (e.ctrlKey || e.metaKey) modifiers.push("CommandOrControl");
    if (e.altKey) modifiers.push("Alt");
    if (e.shiftKey) modifiers.push("Shift");

    const key = e.key.toUpperCase();

    // Ignore if only a modifier was pressed
    if (["CONTROL", "ALT", "SHIFT", "META", "OS"].includes(key)) return;

    // Normalizing F keys and other special keys
    let finalKey = e.key;
    if (e.key === " ") finalKey = "Space";
    else if (e.key.length === 1) finalKey = e.key.toUpperCase();

    // Construct shortcut string
    const shortcutStr = [...modifiers, finalKey].join("+");

    try {
      await invoke("set_shortcut", { shortcutStr });
      hotkeyDisplay.innerText = shortcutStr;
      statusText.innerText = "Shortcut updated";
    } catch (err: any) {
      statusText.innerText = `Error: ${err}`;
      await refreshUI(); // Revert to current setting
    } finally {
      isRecordingHotkey = false;
      hotkeyDisplay.classList.remove("ring-2", "ring-primary", "animate-pulse");
    }
  });
});
