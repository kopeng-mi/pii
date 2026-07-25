# LLM Stats API Reference

## Global Settings
- **Base URL:** `https://api.llm-stats.com/stats/v1`
- **Auth:** `Authorization: Bearer YOUR_API_KEY` (Keys start with `ze_`)

## Rate Limits
| Endpoint | Limit |
|----------|-------|
| `/v1/models/{id}`, `/v1/rankings` | 120 / min |
| `/v1/models`, `/v1/benchmarks`, `/v1/updates` | 60 / min |
| `/v1/scores` | 30 / min |
*(HTTP 429 on limit hit, includes `Retry-After` header)*

## Standard Objects

### Error Envelope
Branch on `code` rather than `message`.
```json
{
  "error": {
    "code": "not_found",
    "message": "Human-readable explanation.",
    "param": "model_id",
    "help_url": "..."
  }
}
```

### Inference Metadata
Returned in model catalogs/details. Shows gateway routability.
```json
"inference": {
  "available": true,
  "endpoint": "https://gateway.llm-stats.com/v1/chat/completions",
  "gateway_model_id": "gpt-5-2025-08-07",
  "openai_compatible": true,
  "supports_streaming": true,
  "supports_tools": true,
  "supports_vision": false,
  "docs_url": "..."
}
```
*(If `available: false`, optional fields are omitted)*

---

## Endpoints

### 1. List Models
`GET /v1/models`
Catalog with metadata, pricing, and category scores.

**Query Parameters (All optional):**
- **Strings:** `organization`, `family`, `modality`, `provider`, `sort`, `cursor`
- **Numbers:** `max_input_price`, `max_output_price`
- **Integers:** `min_context`, `limit` (default: 50, range: 1-200)
- **Booleans:** `open_weight`
- **Dates (String):** `released_after`, `released_before`

**Response (200 OK):**
```json
{
  "models": [
    {
      "id": "<string>",
      "name": "<string>",
      "description": "<string>",
      "organization": { "id": "<string>", "name": "<string>" },
      "license": { "id": "<string>", "name": "<string>", "allow_commercial": true },
      "open_weight": true,
      "model_type": "<string>",
      "modalities": ["<string>"],
      "providers": [
        {
          "provider_id": "<string>",
          "provider_name": "<string>",
          "status": "<string>",
          "input_price_per_m": 123,
          "output_price_per_m": 123
        }
      ],
      "top_scores": {},
      "inference": { /* see Inference Metadata */ },
      "created_at": "2023-11-07T05:31:56Z",
      "updated_at": "2023-11-07T05:31:56Z",
      "source": "<string>",
      "url": "<string>",
      "family": { "id": "<string>", "name": "<string>" },
      "context_window": 123,
      "param_count": 123,
      "release_date": "2023-12-25"
    }
  ],
  "total": 123,
  "next_cursor": "<string>"
}
```

### 2. Get Model
`GET /v1/models/{model_id}`
Full model detail including scores and sources.

**Path Parameters:**
- `model_id` (string, required)

**Response (200 OK):**
*(Same base fields as List Models, plus `scores` and `sources`)*
```json
{
  "id": "<string>",
  ...
  "scores": [
    {
      "benchmark_id": "<string>",
      "benchmark_name": "<string>",
      "score": 123,
      "max_score": 123,
      "is_self_reported": true,
      "verified_by_llmstats": true,
      "scored_at": "2023-11-07T05:31:56Z",
      "category": "<string>",
      "description": "<string>",
      "normalized_score": 123,
      "rank": 123,
      "source_url": "<string>"
    }
  ],
  "sources": {
    "api_ref": "<string>",
    "paper": "<string>",
    "weights": "<string>",
    "repo": "<string>"
  }
}
```

### 3. List Benchmarks
`GET /v1/benchmarks`
All benchmarks with categories and model counts.

**Query Parameters (All optional):**
- **Strings:** `category`
- **Booleans:** `verified`

**Response (200 OK):**
```json
{
  "benchmarks": [
    {
      "id": "<string>",
      "name": "<string>",
      "categories": ["<string>"],
      "modality": "<string>",
      "max_score": 123,
      "language": "<string>",
      "verified": true,
      "model_count": 123,
      "source": "<string>",
      "url": "<string>",
      "description": "<string>",
      "paper_link": "<string>",
      "implementation_link": "<string>"
    }
  ]
}
```

### 4. List Scores
`GET /v1/scores`
Score matrix across models and benchmarks.

**Query Parameters (All optional):**
- **Strings:** `model`, `benchmark`, `category`, `sort`, `cursor`
- **Numbers:** `min_score`, `max_score`
- **Dates (Date-time string):** `scored_after`, `scored_before`
- **Booleans:** `verified_only`
- **Integers:** `limit` (default: 100, range: 1-500)

**Response (200 OK):**
```json
{
  "scores": [
    {
      "model_id": "<string>",
      "model_name": "<string>",
      "organization": "<string>",
      "benchmark_id": "<string>",
      "benchmark_name": "<string>",
      "score": 123,
      "max_score": 123,
      "is_self_reported": true,
      "verified": true,
      "scored_at": "2023-11-07T05:31:56Z",
      "source": "<string>",
      "url": "<string>",
      "category": "<string>",
      "normalized_score": 123
    }
  ],
  "total": 123,
  "next_cursor": "<string>"
}
```

### 5. Get Rankings
`GET /v1/rankings`
TrueSkill rankings by category.

**Query Parameters:**
- `category` (string, **required**)
- `limit` (integer, optional, default: 10, range: 1-50)

**Response (200 OK):**
```json
{
  "category": "<string>",
  "ranked_at": "2023-11-07T05:31:56Z",
  "models": [
    {
      "rank": 123,
      "model_id": "<string>",
      "model_name": "<string>",
      "organization": "<string>",
      "score": 123,
      "conservative_rating": 123,
      "open_weight": true,
      "benchmarks_evaluated": 123,
      "source": "<string>",
      "url": "<string>",
      "min_input_price": 123
    }
  ],
  "method": "trueskill"
}
```

### 6. List Updates
`GET /v1/updates`
Recently added models.

**Query Parameters:**
- `days` (integer, **required**, range: 1-30)
- `limit` (integer, optional, default: 50, range: 1-200)

**Response (200 OK):**
```json
{
  "days": 123,
  "models": [
    {
      "id": "<string>",
      "name": "<string>",
      "organization": { "id": "<string>", "name": "<string>" },
      "model_type": "<string>",
      "modalities": ["<string>"],
      "open_weight": true,
      "added_at": "2023-11-07T05:31:56Z",
      "source": "<string>",
      "url": "<string>",
      "context_window": 123,
      "release_date": "2023-12-25"
    }
  ],
  "total": 123
}
```
