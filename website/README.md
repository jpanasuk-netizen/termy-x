# website

Run development server:

```bash
bun run dev
```

Build:

```bash
bun run build
```

The build uses the Nitro `bun` preset by default (see `vite.config.ts`), which
the `Dockerfile` serves via `bun run start`.

## Deploying to Vercel

Deployment is configured in `vercel.json`. It sets `NITRO_PRESET=vercel` at
build time, so the build emits Vercel's Build Output API artifacts to
`.vercel/output`, which Vercel detects automatically.

When importing the repo into Vercel, set the project **Root Directory** to
`website`. No other configuration is required — `vercel.json` provides the
install and build commands.

To reproduce the Vercel build locally:

```bash
NITRO_PRESET=vercel bun run build
```
