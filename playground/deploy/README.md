# ADK Playground — AWS Deployment

Deploys the playground on an EC2 Spot Instance (ARM Graviton) for ~$12-14/month.

## Architecture

- **t4g.medium** spot instance (2 vCPU, 4GB RAM, ARM64)
- Auto Scaling Group (size=1) for automatic spot replacement
- Elastic IP for stable address across spot interruptions
- Caddy reverse proxy (auto HTTPS if domain provided)
- Public mode only (registered examples, no arbitrary code)
- Pre-warmed Rust compilation cache

## Pricing

| Component | Monthly Cost |
|-----------|-------------|
| t4g.medium spot (~$0.015/hr) | ~$10-12 |
| EBS 20GB gp3 | ~$1.60 |
| Elastic IP (attached) | Free |
| **Total** | **~$12-14** |

vs On-demand: ~$26/mo (55-60% savings)

## Deploy

```bash
export GOOGLE_API_KEY=your-gemini-key
./deploy.sh

# With custom domain + SSH access:
./deploy.sh --domain playground.example.com --key-pair my-key

# Different region:
./deploy.sh --region us-west-2
```

## Tear Down

```bash
./deploy.sh --destroy
```

## Notes

- First boot takes ~10-15 minutes (Rust toolchain install + compilation)
- Spot interruptions are rare for t4g (<5%) and the ASG auto-replaces
- The Elastic IP stays fixed even when the instance is replaced
- Domain requires DNS pointed to the Elastic IP before deploy
