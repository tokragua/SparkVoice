import os
import requests

def download_model(model_name="tiny.en"):
    url = f"https://huggingface.co/distil-whisper/distil-small.en/resolve/main/ggml-tiny.en.bin"
    # Note: Correcting URL if needed. Standard ggml models are often at:
    # https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin
    url = f"https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{model_name}.bin"
    
    output_path = f"src-tauri/ggml-{model_name}.bin"
    
    if os.path.exists(output_path):
        print(f"Model {model_name} already exists at {output_path}")
        return

    print(f"Downloading {model_name} model...")
    response = requests.get(url, stream=True)
    response.raise_for_status()
    
    with open(output_path, "wb") as f:
        for chunk in response.iter_content(chunk_size=8192):
            f.write(chunk)
    print(f"Downloaded {model_name} to {output_path}")

if __name__ == "__main__":
    download_model()
