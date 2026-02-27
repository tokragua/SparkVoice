import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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
  const statusText = document.getElementById("status-text") as HTMLElement;

  const modelList = document.getElementById("model-list") as HTMLElement;
  const modelStatus = document.getElementById("model-status") as HTMLElement;

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
      const settings: any = await invoke("get_settings");
      const availableModels: any[] = await invoke("get_available_models");
      const downloadedModels: string[] = await invoke("get_downloaded_models");

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
        li.innerHTML = `
                    <span>${LANGUAGE_NAMES[lang] || lang} (${lang})</span>
                    <button class="remove-btn" data-lang="${lang}">×</button>
                `;
        myLanguagesList.appendChild(li);
      });

      // Populate models list
      modelList.innerHTML = "";
      availableModels.forEach((model: any) => {
        const isDownloaded = downloadedModels.includes(model.name);
        const isSelected = settings.selected_model === model.name;

        const modelDiv = document.createElement("div");
        modelDiv.className = `model-item ${isSelected ? "selected" : ""}`;
        modelDiv.innerHTML = `
          <div class="model-info">
            <span class="model-name">${model.name}</span>
            <span class="model-meta">${model.size} • ${model.description}</span>
          </div>
          <div class="model-actions">
            ${isSelected ? '<span class="status-pill">Active</span>' : ""}
            ${!isDownloaded ? `<button class="download-btn" data-model="${model.name}">Download</button>` : ""}
            ${isDownloaded && !isSelected ? `<button class="use-btn" data-model="${model.name}">Select</button>` : ""}
            ${isDownloaded ? `<button class="delete-btn" data-model="${model.name}">Delete</button>` : ""}
          </div>
        `;
        modelList.appendChild(modelDiv);
      });

      // Populate device and microphone
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

  // Event Listeners
  languageSelect.addEventListener("change", async () => {
    await invoke("set_language", { lang: languageSelect.value });
    statusText.innerText = "Language updated";
  });

  addLanguageBtn.addEventListener("click", async () => {
    const lang = availableLanguagesSelect.value;
    if (lang) {
      await invoke("add_language", { lang });
      await refreshUI();
      availableLanguagesSelect.value = "";
      statusText.innerText = "Language added";
    }
  });

  myLanguagesList.addEventListener("click", async (e) => {
    const target = e.target as HTMLElement;
    if (target.classList.contains("remove-btn")) {
      const lang = target.getAttribute("data-lang");
      if (lang) {
        await invoke("remove_language", { lang });
        await refreshUI();
        statusText.innerText = "Language removed";
      }
    }
  });

  modelList.addEventListener("click", async (e) => {
    const target = e.target as HTMLElement;
    if (target.tagName === "BUTTON") {
      const model = target.getAttribute("data-model");
      if (!model) return;

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
    await invoke("set_device", { device: deviceSelect.value });
    statusText.innerText = `Switched to ${deviceSelect.value.toUpperCase()}`;
  });
});
