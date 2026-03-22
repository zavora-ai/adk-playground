# ADK-Rust Playground — AWS Deployment

Deploys the playground on an EC2 Spot Instance (ARM Graviton) for ~$12-14/month.

## Architecture

- **t4g.medium** spot instance (2 vCPU, 4GB RAM, ARM64)
- Auto Scaling Group (size=1) for automatic spot replacement
- Elastic IP for stable address across spot interruptions
- Caddy reverse proxy with auto-HTTPS
- Public mode only (registered examples, no arbitrary code)
- Pre-warmed Rust compilation cache

## Auto-Deploy

Two mechanisms keep the instance in sync with `main`:

1. **GitHub Actions** (`.github/workflows/deploy.yml`) — on every push to `main` that touches `playground/`, SSMs into the running instance to pull + rebuild + restart. Requires `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` repo secrets.

2. **Systemd update service** (`adk-playground-update.service`) — runs `git pull && cargo build --release` as a oneshot before the playground starts. Handles spot replacement automatically — every new instance boots with the latest code.

## Pricing

| Component | Monthly Cost |
|-----------|-------------|
| t4g.medium spot (~$0.015/hr) | ~$10-12 |
| EBS 30GB gp3 | ~$2.40 |
| Elastic IP (attached) | Free |
| **Total** | **~$12-14** |

## Deploy

```bash
export GOOGLE_API_KEY=your-gemini-key
./deploy.sh
```

## Tear Down

```bash
./deploy.sh --destroy
```

## Notes

- First boot takes ~15-20 min (Rust toolchain + full compilation + example prebuild)
- Subsequent restarts: ~1 min (git pull + incremental build, examples loaded from cache)
- Spot interruptions are rare for t4g (<5%) and the ASG auto-replaces
- The Elastic IP stays fixed even when the instance is replaced
