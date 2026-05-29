# Email classification eval

Evaluates the [`distil-labs/distil-email-classifier`](https://huggingface.co/distil-labs/distil-email-classifier)
Hugging Face model against primary-inbox emails from the prod DB. The
model is a Qwen3-0.6B distilled into a 10-way classifier:

> Billing · Newsletter · Work · Personal · Promotional ·
> Security · Shipping · Travel · Spam · Other

Reports are written to `reports/evaluations/email_classification/`.

## One-time setup

The model is not on the Ollama registry — pull from HF and register
locally:

```bash
# 1. Download the GGUF + Modelfile (~3.5 GB)
hf download distil-labs/distil-email-classifier --local-dir ./distil-email-classifier

# 2. Register with Ollama under the tag `email-classifier`
cd ./distil-email-classifier
ollama create email-classifier -f Modelfile

# 3. Smoke-test
ollama run email-classifier "Subject: Your Amazon order shipped"
```

## Run the eval

From the repo root:

```bash
# Default: 100 newest primary-inbox emails for the configured account, no judge
cargo run --features eval --example email_classification_eval -- \
  --account alex@example.com

# Override account / limit / model
cargo run --features eval --example email_classification_eval -- \
  --account alex@example.com --limit 100 --model email-classifier

# Enable the OpenRouter LLM-as-judge for reasonableness scoring
export OPENROUTER_API_KEY=…
cargo run --features eval --example email_classification_eval -- --use-judge
```
