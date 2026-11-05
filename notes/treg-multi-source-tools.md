# Treg multi-source tools for Reply Guy

Discovered 2026-11-05 against Jeremy's Treg org (token not stored here).
Callable form: `POST https://treg.to/call/<tool-id>` with header `X-Treg-Token`.
Catalog HTML lists many dotted ids; **only some are callable for this org**. Prefer `anyapi.*` ids that return schema errors or data (not `no tool in this org`).

## Wired (callable + useful for trends/search)

| Source | Tool id | Input | Notes / sample costUsd |
|--------|---------|-------|------------------------|
| X | `anyapi.x.search.posts` | `query`, `limit`, `queryType` (Top/Latest) | ~0.0004–0.00075 |
| X trends | `anyapi.twitter.trends` | `limit`, optional `location` | names only; ~0.0001+ |
| Reddit | `anyapi.reddit.search` | **required** `query`; optional sort/timeframe/cursor (**no `limit`**) | ~0.00035 |
| Reddit trending | `anyapi.reddit.trending_posts` | optional (empty `{}` ok); `limit` allowed | posts feed |
| TikTok | `anyapi.tiktok.search.videos` | **required** `query` (**no `limit`**) | ~0.0006 |
| TikTok trends | `anyapi.tiktok.trending_hashtags` | `{}` ok | hashtag list |
| Instagram | `anyapi.instagram.search.reels` | **required** `query` | ~0.0011 |
| Instagram | `anyapi.instagram.hashtag.posts` | **required** `hashtag` | ~0.0012 |
| YouTube | `anyapi.youtube.search` | **required** `query` | ~0.0005 |
| YouTube | `anyapi.youtube.trending_shorts` | `{}` ok | shorts list |
| LinkedIn | `anyapi.linkedin.search.posts` | **required** `query` | ~0.0012 |
| Facebook | `anyapi.facebook.search_posts` | **required** `query`; optional `limit` | schema confirmed |

Auth/me: `GET https://treg.to/auth/me` (free).

## Wanted but NOT callable for this org (catalog anchors only)

| Source | Catalog ids seen | Call result |
|--------|------------------|-------------|
| Hacker News | `hackernews.stories.front`, `hackernews.search.stories` | 404 no tool in this org / no anyapi.* |
| Product Hunt | `producthunt.launches.today` (+ `_detailed`) | 404 no tool in this org |
| Threads | `threads.search.posts` | 404; no anyapi.threads.search |
| Bluesky | `bluesky.user.posts`, `bluesky.post.detail` | no search; anyapi.bluesky.* missing |
| TikTok explore (non-anyapi) | `tiktok.feed.explore` etc. | use anyapi.* instead |
| YouTube official GET | `youtube.youtube.search.videos` | requires GET + query `part`/`q`; prefer `anyapi.youtube.search` |

## Response shapes (parsers)

Common envelope: `{ "output": { "data": { ... } }, "costUsd": number }`.
Error-not-empty: `{ "detail": "..." }` or `{ "error": "..." }` — never treat as zero hits.

- Reddit search/trending: `output.data.posts[]` — `id`, `author`, `title`, `selftext`, `permalink`, `url`, `score`, `numComments`, `subreddit`, `createdUtc`
- TikTok videos: `output.data.videos[]` — `id`, `author`, `authorName`, `caption`, `url`, `likes`, `shares`, `comments`, `views`, `createdUtc`
- Instagram reels/posts: `output.data.reels[]` or `.posts[]` — `id`, `username`, `displayName`, `caption`, `url`, `likes`, `comments`, `views?`, `createdUtc`
- YouTube search: `output.data.videos[]` — `id`, `channel`, `title`, `url`, `views`, `createdUtc`
- YouTube shorts: `output.data.shorts[]` — similar
- LinkedIn: `output.data.posts[]` — `id`, `authorName`, `text`, `url`, `reactionCount`, `commentCount`, `createdUtc`
- X search: `output.data.items[]` — existing parser (`authorUsername`, `text`, …)
- X trends: `output.data.items[]` — `name`, `rank`, `url`

## UI note

Reply Guy tabs filter by source badge. Non-X hits use Quote-on-X (compose with text + URL attribution). X hits keep Reply/Confirm.
