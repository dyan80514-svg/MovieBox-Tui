# MovieBox web companion

A standalone, no-build static app. The Rust application and its configuration are unchanged.

## Run locally

From the repository root:

```sh
python3 -m http.server 8080 --directory web
```

Open `http://localhost:8080`. Serve over HTTP rather than opening `index.html` as a file because the JavaScript uses ES modules.

## GitHub Pages

Publish the contents of `web/` as the static site artifact. If publishing the entire repository instead, visit `/<repo>/web/`. All bundled asset references are relative, so either layout works under a repository sub-path. No npm packages, build step, or backend are required. This change does not modify any deployment workflows.

## API integration

The app reuses the Cinemeta addon already configured in `src/providers/addons/models.rs`:

- Manifest: `https://v3-cinemeta.strem.io/manifest.json`
- Popular movies: `https://v3-cinemeta.strem.io/catalog/movie/top.json`
- Popular series: `https://v3-cinemeta.strem.io/catalog/series/top.json`
- Top-rated catalogs: `/catalog/{type}/imdbRating.json`
- Search: `/catalog/{type}/top/search={encoded_query}.json`
- Details: `/meta/{type}/{id}.json`

These GET paths and response wrappers match `src/providers/addons/client.rs`. Search encoding matches Rust's `NON_ALPHANUMERIC` percent encoding. The metadata adapter preserves the repository's composite IDs and ports the catalog, detail, season, and episode fields, including alternative names, numeric ratings, and string or object lists.

The native client's User-Agent is browser-controlled and is not spoofed. No authentication or invented API keys are required.

Requests have a 12-second timeout per attempt. Network/CORS failures automatically retry through `https://corsproxy.io/?url=<encoded URL>`. HTTP errors and malformed JSON are reported instead of silently becoming empty results. Catalogs may partially succeed; the interface reports this and provides a retry button. In-memory responses are cached for five minutes.

Genre filters apply to the fetched catalog. Show more reveals additional loaded results; it does not invent an upstream pagination contract. Movie and series catalogs are interleaved on Discover.

## Features and scope

- Live popular and top-rated movie/series catalogs.
- Submitted search, content-type filters, and loaded-title genre filtering.
- Dynamic spotlight, poster grid, and textual image fallbacks.
- Accessible native dialogs with synopsis, ratings, credits, and season/episode metadata.
- Browser-local favorites with cross-tab updates and storage-failure handling.
- Loading skeletons, empty states, partial failures, request cancellation, and retry controls.
- Responsive navigation and layouts; keyboard search shortcut `/`; Escape closes dialogs.

Cinemeta supplies catalog metadata, not video streams. The web app deliberately does not present a fake play button, fabricated media URLs, or unsupported local-player launches. Installation links lead to the Rust app for playback, downloads, and its other providers. MovieBox signing, private BDIX services, and native player integration are not implemented in this static companion.

Favorites are stored separately from the Rust application's files. External providers receive catalog and image requests. The CORS fallback proxy receives the requested URL, including search terms. No analytics or telemetry scripts are included.

## Manual verification

1. Serve `web/` and confirm that movie and series cards load from Cinemeta.
2. Search for a title containing spaces or punctuation and inspect the encoded catalog request.
3. Switch between Movies, TV shows, and Top rated. Change genre and use Show more.
4. Open a movie detail and a series detail; change seasons and inspect episode metadata.
5. Save a title, open My list, reload, and remove it. Confirm changes also appear in another tab.
6. Block a poster URL to verify the text fallback.
7. Block Cinemeta requests and inspect the automatic proxy request. Block both destinations and verify the error and retry state.
8. Change views while requests are pending and confirm stale results do not replace the current view.
9. Check keyboard navigation, dialog focus, Escape, and the `/` shortcut.
10. Test at 375px and desktop widths, and serve beneath a nested repository path to verify relative assets.

These checks are provided for deployment verification; live network and browser tests were not executed while generating these files.
