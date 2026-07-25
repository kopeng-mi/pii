# Artificial Analysis API Reference

## Global Settings
- **Base URL:** `https://artificialanalysis.ai/api/v2`
- **Auth:** `x-api-key: YOUR_API_KEY` (Get key by creating an account on the insights platform)
- **Attribution Required:** Link to `https://artificialanalysis.ai/`
- **Identifiers:** Use model `id` and `model_creator.id` (stable), not `slug` or `name`.

## Free Data API Overview
- **Rate Limit:** 1,000 requests / day (cache responses, do not include keys in client-side code).
- **Focus:** Primary metrics (intelligence, speed, pricing) from independent benchmarks.

### 1. LLMs Endpoint
`GET /data/llms/models`
Returns benchmark scores, pricing, and speed data for LLMs.

**Response (200 OK):**
```json
{
  "status": 200,
  "prompt_options": {
    "parallel_queries": 1,
    "prompt_length": "medium"
  },
  "data": [
    {
      "id": "2dad8957...",
      "name": "o3-mini",
      "slug": "o3-mini",
      "model_creator": { "id": "e67...", "name": "OpenAI", "slug": "openai" },
      "evaluations": {
        "artificial_analysis_intelligence_index": 62.9,
        "artificial_analysis_coding_index": 55.8,
        "artificial_analysis_math_index": 87.2,
        "mmlu_pro": 0.791,
        "gpqa": 0.748,
        "hle": 0.087,
        "livecodebench": 0.717,
        "scicode": 0.399,
        "math_500": 0.973,
        "aime": 0.77
      },
      "pricing": {
        "price_1m_blended_3_to_1": 1.925,
        "price_1m_input_tokens": 1.1,
        "price_1m_output_tokens": 4.4
      },
      "median_output_tokens_per_second": 153.831,
      "median_time_to_first_token_seconds": 14.939,
      "median_time_to_first_answer_token": 14.939
    }
  ]
}
```

### 2. Media Endpoints
Elo ratings for various media models. All return standard `data` array with `elo`, `rank`, `ci95`, `appearances`, and `release_date`. 
Endpoints supporting `?include_categories=true` provide a breakdown of Elo scores per category inside a `categories` array.

- **Text-to-Image:** `GET /data/media/text-to-image` (Supports `include_categories=true`)
- **Image Editing:** `GET /data/media/image-editing`
- **Text-to-Speech:** `GET /data/media/text-to-speech`
- **Text-to-Video:** `GET /data/media/text-to-video` (Supports `include_categories=true`)
- **Image-to-Video:** `GET /data/media/image-to-video` (Supports `include_categories=true`)

**Media Example Response (`GET /data/media/text-to-image?include_categories=true`):**
```json
{
  "status": 200,
  "include_categories": true,
  "data": [
    {
      "id": "dall-e-3",
      "name": "DALL·E 3",
      "slug": "dall-e-3",
      "model_creator": { "id": "openai", "name": "OpenAI" },
      "elo": 1250,
      "rank": 1,
      "ci95": "-5/+5",
      "appearances": 5432,
      "release_date": "2025-04",
      "categories": [
        {
          "style_category": "General & Photorealistic",
          "subject_matter_category": "People: Portraits",
          "elo": 1280,
          "ci95": "-5/+5",
          "appearances": 1234
        }
      ]
    }
  ]
}
```

---

## CritPt Benchmark Evaluation API
Gateway for evaluating code generation submissions against the [CritPt](https://critpt.com/) private evaluation set. Attribution to CritPt is required.

- **Rate Limit:** 10 requests / 24 hours. Rate limit headers included: `X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset`. HTTP 429 returns a `Retry-After` header.

### Batch Evaluation
`POST /api/v2/critpt/evaluate`
Submit code generation submissions for grading. *Must include all problems in the public set.* Can take substantial time to process.

**Request Body:**
```json
{
  "submissions": [
    {
      "problem_id": "Challenge_1_main",
      "generated_code": "```python\ndef solution():\n    return 42\n```",
      "model": "gpt-5",
      "generation_config": {
        "use_golden_for_prev_steps": false,
        "parsing": false,
        "multiturn_with_answer": false,
        "use_python": false,
        "use_web_search": false
      }
    }
  ],
  "batch_metadata": {}
}
```
*(Optionally include `"messages": [...]` in the submission objects)*

**Response (200 OK):**
```json
{
  "accuracy": 0.0,
  "timeout_rate": 0.0,
  "server_timeout_count": 0,
  "judge_error_count": 0
}
```

**Error Responses:**
- `400` Invalid request body
- `401` Invalid/missing API key
- `429` Rate limit exceeded
- `502` Invalid response from evaluation system
- `504` Evaluation timeout