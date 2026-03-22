#!/bin/bash
set -euo pipefail

# ADK-Rust Playground — Deploy to AWS Spot Instance
# Prerequisites: AWS CLI configured, GOOGLE_API_KEY set
#
# Usage:
#   export GOOGLE_API_KEY=your-key
#   ./deploy.sh
#   ./deploy.sh --domain playground.example.com --key-pair my-key
#   ./deploy.sh --region us-west-2

STACK_NAME="adk-playground"
REGION="${AWS_REGION:-us-east-1}"
INSTANCE_TYPE="t4g.medium"
SPOT_MAX_PRICE="0.025"
DOMAIN=""
KEY_PAIR=""
REPO="https://github.com/zavora-ai/adk-playground.git"

while [[ $# -gt 0 ]]; do
  case $1 in
    --domain)    DOMAIN="$2"; shift 2 ;;
    --key-pair)  KEY_PAIR="$2"; shift 2 ;;
    --region)    REGION="$2"; shift 2 ;;
    --repo)      REPO="$2"; shift 2 ;;
    --destroy)
      echo "Destroying stack ${STACK_NAME}..."
      aws cloudformation delete-stack --stack-name "$STACK_NAME" --region "$REGION"
      aws cloudformation wait stack-delete-complete --stack-name "$STACK_NAME" --region "$REGION"
      echo "Done."
      exit 0 ;;
    -h|--help)
      echo "Usage: ./deploy.sh [--domain DOMAIN] [--key-pair KEY] [--region REGION] [--repo URL] [--destroy]"
      exit 0 ;;
    *) echo "Unknown: $1"; exit 1 ;;
  esac
done

if [ -z "${GOOGLE_API_KEY:-}" ]; then
  echo "Error: export GOOGLE_API_KEY first"
  exit 1
fi

echo "╔══════════════════════════════════════════╗"
echo "║   ADK-Rust Playground — AWS Deployment   ║"
echo "╚══════════════════════════════════════════╝"
echo ""
echo "  Region:    ${REGION}"
echo "  Instance:  ${INSTANCE_TYPE} (ARM Graviton)"
echo "  Spot bid:  \$${SPOT_MAX_PRICE}/hr"
echo "  Domain:    ${DOMAIN:-none (HTTP on IP)}"
echo ""

# Show current spot prices
echo "Current spot prices for ${INSTANCE_TYPE} in ${REGION}:"
aws ec2 describe-spot-price-history \
  --region "$REGION" \
  --instance-types "$INSTANCE_TYPE" \
  --product-descriptions "Linux/UNIX" \
  --start-time "$(date -u +%Y-%m-%dT%H:%M:%S)" \
  --query 'SpotPriceHistory[*].[AvailabilityZone,SpotPrice]' \
  --output table 2>/dev/null || echo "  (couldn't fetch — check AWS CLI config)"
echo ""

echo "Estimated monthly cost: ~\$12-14 (spot + EBS)"
echo "vs On-demand: ~\$26/mo"
echo ""

read -p "Deploy? (y/N) " -n 1 -r
echo
[[ ! $REPLY =~ ^[Yy]$ ]] && { echo "Cancelled."; exit 0; }

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo ""
echo "Deploying CloudFormation stack..."
aws cloudformation deploy \
  --region "$REGION" \
  --stack-name "$STACK_NAME" \
  --template-file "${SCRIPT_DIR}/cloudformation.yaml" \
  --capabilities CAPABILITY_IAM \
  --parameter-overrides \
    GoogleApiKey="$GOOGLE_API_KEY" \
    SpotMaxPrice="$SPOT_MAX_PRICE" \
    InstanceType="$INSTANCE_TYPE" \
    DomainName="$DOMAIN" \
    KeyPairName="$KEY_PAIR" \
    GitHubRepo="$REPO" \
  --tags Key=Project,Value=adk-playground

echo ""
echo "=== Deployed ==="
aws cloudformation describe-stacks \
  --region "$REGION" \
  --stack-name "$STACK_NAME" \
  --query 'Stacks[0].Outputs[*].[OutputKey,OutputValue]' \
  --output table

echo ""
echo "The instance is booting and compiling (~10-15 min for first setup)."
echo "Check progress:"
echo "  ssh ec2-user@<IP> 'tail -f /var/log/adk-playground-setup.log'"
echo ""
echo "Tear down:"
echo "  ./deploy.sh --destroy"
