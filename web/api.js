// Endpoint contracts: src/providers/addons/client.rs and models.rs.
export const MANIFEST_URL = 'https://v3-cinemeta.strem.io/manifest.json';
const BASE_URL = MANIFEST_URL.slice(0, -'/manifest.json'.length);
const cache = new Map();

// Match Rust percent_encoding::NON_ALPHANUMERIC, including punctuation.
export function encodeParam(value) {
  return Array.from(new TextEncoder().encode(String(value))).map(byte =>
    (byte >= 48 && byte <= 57) || (byte >= 65 && byte <= 90) || (byte >= 97 && byte <= 122)
      ? String.fromCharCode(byte) : '%' + byte.toString(16).toUpperCase().padStart(2, '0')
  ).join('');
}

async function attempt(url, signal) {
  const controller = new AbortController();
  const abort = () => controller.abort();
  if (signal?.aborted) throw new DOMException('Cancelled', 'AbortError');
  signal?.addEventListener('abort', abort, { once: true });
  const timer = setTimeout(abort, 12000);
  try {
    // Browsers control User-Agent; the native Addon-Client UA cannot reliably be set here.
    const response = await fetch(url, { signal: controller.signal, credentials: 'omit' });
    if (!response.ok) throw new Error(`Cinemeta returned HTTP ${response.status}.`);
    return await response.json();
  } finally {
    clearTimeout(timer);
    signal?.removeEventListener('abort', abort);
  }
}

async function request(url, signal) {
  if (signal?.aborted) throw new DOMException('Cancelled', 'AbortError');
  const cached = cache.get(url);
  if (cached && Date.now() - cached.time < 300000) return cached.data;
  let data;
  try {
    data = await attempt(url, signal);
  } catch (error) {
    if (signal?.aborted) throw new DOMException('Cancelled', 'AbortError');
    // Fetch exposes CORS failures as TypeError; network failures have the same shape.
    if (!(error instanceof TypeError) && error.name !== 'AbortError') throw error;
    try {
      data = await attempt('https://corsproxy.io/?url=' + encodeURIComponent(url), signal);
    } catch (retryError) {
      if (signal?.aborted) throw new DOMException('Cancelled', 'AbortError');
      throw new Error('Could not reach Cinemeta directly or through the fallback proxy. Please try again.');
    }
  }
  if (cache.size >= 80) cache.delete(cache.keys().next().value);
  cache.set(url, { data, time: Date.now() });
  return data;
}

export function safeImage(value) {
  try {
    const url = new URL(value);
    return url.protocol === 'https:' ? url.href : '';
  } catch { return ''; }
}

function list(value) {
  if (Array.isArray(value)) return value.map(item => typeof item === 'string' ? item : item?.name).filter(Boolean);
  if (typeof value === 'string') return value.split(',').map(item => item.trim()).filter(Boolean);
  return [];
}

// CatalogItem and MediaDetails use the same field names as the Rust provider models.
export function adaptMeta(raw, typeHint = 'movie') {
  const videos = Array.isArray(raw.videos) ? raw.videos : [];
  const type = ['series', 'tv', 'anime'].includes(String(raw.type || typeHint).toLowerCase()) || videos.length ? 'series' : 'movie';
  const id = String(raw.id || '');
  const yearRaw = String(raw.releaseInfo ?? raw.year ?? raw.released ?? '');
  const seasons = new Map();
  for (const video of videos) {
    const season = Number(video.season ?? 1);
    const number = Number(video.episode ?? video.number ?? 1);
    if (!Number.isInteger(season) || season < 0 || !Number.isInteger(number) || number < 0) continue;
    if (!seasons.has(season)) seasons.set(season, new Map());
    if (!seasons.get(season).has(number)) seasons.get(season).set(number, {
      season, number, title: video.title || video.name || null,
      overview: video.overview || video.description || null
    });
  }
  const firstList = (...values) => values.map(list).find(items => items.length) || [];
  return {
    id: { provider: 'addons', value: `${type}:${id}` },
    source_id: id,
    title: String(raw.name?.trim() || raw.title || 'Unknown title'),
    media_type: type,
    year: yearRaw.match(/[12][0-9]{3}/)?.[0] || null,
    poster_url: safeImage(raw.poster || raw.cover || raw.background),
    background: safeImage(raw.background),
    description: raw.description || raw.overview || raw.synopsis || null,
    imdb_rating: raw.imdbRating != null ? String(raw.imdbRating) : raw.rating != null ? String(raw.rating) : null,
    duration: raw.runtime ? String(raw.runtime) : null,
    genres: firstList(raw.genres, raw.genre),
    director: firstList(raw.director, raw.directors, raw.writers, raw.writer).join(', ') || null,
    stars: firstList(raw.cast, raw.stars).join(', ') || null,
    seasons: [...seasons.entries()].sort((a, b) => a[0] - b[0]).map(([number, episodes]) => ({
      number, episodes: [...episodes.values()].sort((a, b) => a.number - b.number)
    })),
    season_count: seasons.size || null,
    tagline: null, prints: null, audios: null, dubs: []
  };
}

export async function fetchCatalog(type, catalog = 'top', query = '', signal) {
  const extra = query.trim() ? `/search=${encodeParam(query.trim())}` : '';
  const url = `${BASE_URL}/catalog/${type}/${query.trim() ? 'top' : catalog}${extra}.json`;
  const raw = await request(url, signal);
  const metas = raw?.metas ?? raw?.items ?? raw?.results ?? (Array.isArray(raw) ? raw : []);
  if (!Array.isArray(metas)) throw new Error('Cinemeta returned an invalid catalog. Please try again.');
  return metas.filter(item => item && item.id).map(item => adaptMeta(item, type));
}

export async function fetchDetails(item, signal) {
  const raw = await request(`${BASE_URL}/meta/${item.media_type}/${encodeParam(item.source_id)}.json`, signal);
  const meta = raw?.meta && typeof raw.meta === 'object' ? raw.meta : raw;
  if (!meta?.id || String(meta.id) !== item.source_id || !(meta.name || meta.title)) {
    throw new Error('Full details are unavailable for this title.');
  }
  return adaptMeta(meta, item.media_type);
}
