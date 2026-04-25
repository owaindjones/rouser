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
| **Inference Engine** | llama.cpp (ghcr.io/ggml-org/llama.cpp:server-vulkan Docker container) |
| **Model Parameters** | 35B total / 3B active per token (MoE architecture)                    |
| **Context Window** | 262,144 tokens trained; 150,000 server context size                   |
| **Vocabulary Size** | 248,320                                                               |

### Hardware Specifications

| Component           | Specification |
|---------------------|--------------|
| **CPU**             | AMD Ryzen 9 5950X (16-core / 32-thread) @ ~3.87 GHz |
| **RAM**             | 64 GB DDR4 total (~30 GB available idle) |
| **GPU 1 (Display)** | NVIDIA GeForce RTX 4070 Ti SUPER (AD103, Ada Lovelace) |
| **GPU 2 (Compute)** | AMD Radeon RX 7900 XT/XTX/GRE (Navi 31, RDNA 3) |
| **Storage**         | WDC SN580 1TB NVMe + WD Blue SN570 2TB NVMe |

### Operating System

| Attribute | Value |
|-----------|-------|
| **OS** | Fedora Linux 43 Workstation Edition |
| **Kernel** | 6.19.12-cachyos x86_64 (PREEMPT) |
| **Architecture** | x86_64 GNU/Linux |

### Inference Server Configuration

- **Container**: `llama-llama-1` (`ghcr.io/ggml-org/llama.cpp:server-vulkan`)
- **Server Alias**: Qwen3
- **Port**: 11433 (host-mapped)
- **Backend**: Vulkan GPU acceleration
- **Batch Size**: 4096 / ubatch-size: 4096
- **KV Cache**: q8_0 quantized, unified KV cache enabled
- **Flash Attention**: Enabled
- **Temperature**: 1.0 | Top-P: 0.95 | Min-P: 0.15 | Repeat Penalty: 1.05
- **Presence Penalty**: 1.5

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
