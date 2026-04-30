# VIBECODED.md

## Agent Attribution

This document credits the AI agent responsible for code written during development.

### Agent Statistics

| Attribute | Value                                                                 |
|-----------|-----------------------------------------------------------------------|
| **Agent Name** | Sisyphus (OhMyOpenCode Orchestrator)                                  |
| **Underlying Model** | Qwen3.6 (UD variant, mixture-of-experts)                              |
| **Environment** | opencode CLI — OhMyOpenCode platform                                  |
| **Model File** | Qwen3.6-35B-A3B-UD-IQ4_XS.gguf (~17 GB, IQ4_XS quantization)          |
| **Inference Engine** | llama.cpp (`ghcr.io/ggml-org/llama.cpp:server-rocm` Docker container) |
| **Model Parameters** | 35B total / 3B active per token (MoE architecture)                    |
| **Context Window** | 262,144 tokens trained; 150,000 server context size                   |
| **Vocabulary Size** | 248,320                                                               |

### Hardware Specifications

| Component           | Specification |
|---------------------|--------------|
| **CPU**             | AMD Ryzen 9 5950X (16-core / 32-thread) @ ~3.87 GHz |
| **RAM**             | 64 GB DDR4 total (~30 GB available idle) |
| **GPU 1 (Compute)** | AMD Radeon RX 7900 XT/XTX/GRE (Navi 31, RDNA 3) |
| **GPU 2 (Display)** | NVIDIA GeForce RTX 4070 Ti SUPER (AD103, Ada Lovelace) |
| **Storage**         | WDC SN580 1TB NVMe + WD Blue SN570 2TB NVMe |

### Operating System

| Attribute | Value |
|-----------|-------|
| **OS** | Fedora Linux 43 Workstation Edition |
| **Kernel** | 7.0.1-cachyos1.fc43.x86_64 (PREEMPT) |
| **Architecture** | x86_64 GNU/Linux |

### Inference Server Configuration

- **Container**: `llama-llama-1` (`ghcr.io/ggml-org/llama.cpp:server-rocm`)
- **Server Alias**: Qwen3
- **Backend**: ROCm GPU acceleration (AMD Navi 31)
- **Model Path**: `/models/Qwen3.6/Qwen3.6-35B-A3B-UD-IQ4_XS.gguf`
- **MMProj Path**: `/models/Qwen3.6/mmproj-F16.gguf` (multimodal projector, F16)
- **Batch Size**: 4096 / ubatch-size: 4096
- **KV Cache**: q8_0 quantized K and V caches separately, unified KV cache enabled (`--kv-unified`)
- **Flash Attention**: Enabled (`--flash-attn on`)
- **Top-K**: 20 | Temperature: 1.0 | Top-P: 0.95 | Min-P: 0.15 | Repeat Penalty: 1.05
- **Presence Penalty**: 1.5
- **Checkpointing**: Every-n-tokens: 1024, ctx-checkpoints: 128 (context shift mode)
- **Parallel Processing**: `--parallel 2`, context-shift enabled
- **Caching**: Prompt caching (`--cache-prompt`), mmap + mlock enabled
- **Cache RAM**: 12288 MB allocated for offload fallback
- **Other Flags**: No warmup, fit disabled, cache-reuse=1, jinja template mode

### Environment Variables (Container)

| Variable | Value | Purpose |
|----------|-------|---------|
| `HSA_OVERRIDE_GFX_VERSION` | 11.0.0 | Overrides ROCm GPU architecture detection for Navi 31 |
| `GGML_VK_VISIBLE_DEVICES` | 0 | llama.cpp device selection (maps to first AMD GPU) |
| `ROCR_VISIBLE_DEVICES` | GPU-75c130d5baf68bb6 | AMD ROCm device UUID |
| `AMD_VULKAN_ICD` | RADV | Vulkan ICD driver for AMD GPUs |
| `ZES_ENABLE_SYSMAN` | 1 | Enable Intel/AMD system manager telemetry |

### Agent Capabilities

The Sisyphus agent operates within the opencode environment with access to:
- Direct filesystem operations (read, write, edit files)
- Shell execution via Bash tool (git, cargo, docker, system commands)  
- Web search and documentation lookup tools
- Parallel task delegation to specialized sub-agents (explore, librarian, oracle, etc.)
- LSP integration for code navigation and diagnostics
- AST-grep patterns for structural code searches

### Code Contribution

**100% of code written by the AI agent.** No human wrote or edited code during this development session. The agent:
- Explored the codebase autonomously
- Identified problems and proposed solutions  
- Made all code modifications
- Wrote documentation from scratch
- Ran tests and verified correctness

All work was performed autonomously using the opencode toolchain without human intervention in code writing or editing.

---

*This file serves solely to attribute AI agent contributions. All project documentation is available in the `docs/` directory.*
