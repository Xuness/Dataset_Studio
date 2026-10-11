"""Kaloscope v1's frozen style branch, without loading its artist classifier.

The model card specifies the public architecture packaged at the pinned
comfyui-kaloscope revision. Only its DINOv3 architecture is an installed runtime
dependency; no ComfyUI imports, custom checkpoint code, or pickle execution.
"""

import torch
from safetensors.torch import load_file
from torch import nn


class Kaloscope(nn.Module):
    def __init__(self, weights):
        super().__init__()
        from kaloscope_dinov3.architecture import build_backbone

        state = load_file(weights, device="cpu")
        self.backbone = build_backbone({"name": "dinov3_vitb16"})
        backbone = {k.removeprefix("backbone."): v for k, v in state.items() if k.startswith("backbone.")}
        self.backbone.load_state_dict(backbone, strict=True)
        projector = {k.removeprefix("projector."): v for k, v in state.items() if k.startswith("projector.")}
        hidden, dimension = projector["0.weight"].shape
        output, second_hidden = projector["2.weight"].shape
        if (dimension, output, second_hidden) != (1536, 256, hidden):
            raise ValueError("unsupported Kaloscope projection dimensions")
        self.projector = nn.Sequential(nn.Linear(dimension, hidden), nn.GELU(), nn.Linear(hidden, output))
        self.projector.load_state_dict(projector, strict=True)
        unexpected = [k for k in state if not k.startswith(("backbone.", "projector.", "head.", "linear_head."))
                      and k not in ("log_temperature", "bias")]
        if unexpected:
            raise ValueError(f"unsupported Kaloscope checkpoint keys: {unexpected[:4]}")
        self.register_buffer("mean", torch.tensor([0.485, 0.456, 0.406]).view(1, 3, 1, 1), persistent=False)
        self.register_buffer("std", torch.tensor([0.229, 0.224, 0.225]).view(1, 3, 1, 1), persistent=False)

    def forward(self, images):
        images = (images.float() / 255.0 - self.mean) / self.std
        tokens = self.backbone.forward_features(images)
        pooled = torch.cat((tokens["x_norm_clstoken"], tokens["x_norm_patchtokens"].mean(1)), dim=-1)
        return self.projector(pooled)
