## Set up
1. Download `model.safetensors` from https://huggingface.co/meta-llama/Llama-3.2-1B/tree/main, (access can be requested here https://huggingface.co/meta-llama/Llama-3.2-1B)
2. Move downloaded `model.safetensors` to `worker/llama3v2-1b`

## Usage info
1. If you don't have CUDA device, you should initialize model with `cuda_device_id` set to `None`
2. If you encounter problems during compilation of candle regarding CUDA try to install `nvidia-cuda-toolkit`