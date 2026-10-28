import { fetchCatalog, fetchDetails, safeImage } from './api.js';

const $ = selector => document.querySelector(selector);
const STORAGE_KEY = 'moviebox-web:favorites:v1';
const state = { view: 'discover', type: 'all', query: '', sort: 'top', genre: '', items: [], visible: 18, loading: false };
let saved = readSaved();
let catalogController;
let detailController;
let spotlight = null;
let selected = null;
let toastTimer;
let generation = 0;

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}
function key(item) { return item.id.value; }
function readSaved() {
  try {
    const raw = JSON.parse(localStorage.getItem(STORAGE_KEY) || '[]');
    return Array.isArray(raw) ? raw.filter(item => item?.id?.provider === 'addons' && typeof item.id.value === 'string' && typeof item.source_id === 'string' && typeof item.title === 'string' && ['movie', 'series'].includes(item.media_type) && Array.isArray(item.genres) && Array.isArray(item.seasons)) : [];
  } catch { return []; }
}
function isSaved(item) { return saved.some(entry => key(entry) === key(item)); }
function toast(message) {
  clearTimeout(toastTimer);
  $('#toast').textContent = message;
  $('#toast').hidden = false;
  toastTimer = setTimeout(() => { $('#toast').hidden = true; }, 3500);
}
function updateSaveButtons() {
  $('#saved-count').textContent = saved.length;
  document.querySelectorAll('[data-save-key]').forEach(button => {
    const active = saved.some(item => key(item) === button.dataset.saveKey);
    button.setAttribute('aria-pressed', String(active));
    button.textContent = button.classList.contains('card-save') ? (active ? '✓' : '+') : (active ? '✓ In my list' : '＋ My list');
    button.setAttribute('aria-label', `${active ? 'Remove from' : 'Add to'} my list: ${button.dataset.title}`);
  });
}
function toggleSave(item) {
  const exists = isSaved(item);
  saved = exists ? saved.filter(entry => key(entry) !== key(item)) : [item, ...saved];
  let persisted = true;
  try { localStorage.setItem(STORAGE_KEY, JSON.stringify(saved)); } catch { persisted = false; }
  updateSaveButtons();
  if (state.view === 'saved') {
    state.items = saved;
    updateGenres();
    renderGrid();
  }
  toast(persisted ? (exists ? 'Removed from your list' : 'Added to your list') : 'Updated for this session. Browser storage is unavailable.');
}
function configureSave(button, item) {
  button.dataset.saveKey = key(item);
  button.dataset.title = item.title;
  button.onclick = () => toggleSave(item);
}
function poster(item, className = '') {
  const container = el('div', 'poster ' + className);
  const fallback = el('div', 'poster-fallback');
  fallback.append(el('b', '', item.title.slice(0, 1).toUpperCase()), el('span', '', item.title));
  fallback.setAttribute('aria-hidden', 'true');
  container.append(fallback);
  const url = safeImage(item.poster_url);
  if (url) {
    const image = el('img');
    image.alt = '';
    image.loading = 'lazy';
    image.decoding = 'async';
    image.referrerPolicy = 'no-referrer';
    image.addEventListener('error', () => image.remove(), { once: true });
    image.src = url;
    container.append(image);
  }
  return container;
}
function rating(item) {
  const value = Number(item.imdb_rating);
  return Number.isFinite(value) && value > 0 && value <= 10 ? value.toFixed(1) : '';
}
function card(item) {
  const article = el('article', 'card');
  const open = el('button', 'poster-button');
  open.setAttribute('aria-label', `View details for ${item.title}`);
  const art = poster(item);
  if (rating(item)) {
    const badge = el('span', 'card-rating');
    badge.append(el('span', 'star', '★ '), document.createTextNode(rating(item)));
    art.append(badge);
  }
  open.append(art, el('span', 'card-title', item.title));
  open.title = item.title;
  open.onclick = () => openDetails(item);
  const save = el('button', 'card-save');
  configureSave(save, item);
  const meta = el('div', 'card-meta');
  meta.append(el('span', '', item.year || 'Year unknown'), el('span', '', item.media_type === 'series' ? 'TV show' : 'Movie'));
  article.append(open, save, meta);
  return article;
}
function showSkeletons() {
  $('#grid').replaceChildren();
  for (let i = 0; i < 12; i++) {
    const skeleton = el('div', 'skeleton');
    skeleton.setAttribute('aria-hidden', 'true');
    skeleton.append(el('div', 'poster'), el('div', 'skeleton-line'), el('div', 'skeleton-line short'));
    $('#grid').append(skeleton);
  }
}
function visibleItems() {
  let items = state.items;
  if (state.type !== 'all') items = items.filter(item => item.media_type === state.type);
  if (state.genre) items = items.filter(item => item.genres.includes(state.genre));
  if (state.view === 'saved' && state.query) items = items.filter(item => item.title.toLocaleLowerCase().includes(state.query.toLocaleLowerCase()));
  return items;
}
function renderGrid() {
  if (state.loading) return;
  const items = visibleItems();
  const count = Math.min(state.visible, items.length);
  $('#grid').replaceChildren(...items.slice(0, count).map(card));
  $('#grid').setAttribute('aria-busy', 'false');
  $('#results-status').textContent = items.length ? `Showing ${count} of ${items.length} loaded titles${state.genre ? ' · ' + state.genre : ''}` : '';
  $('#empty').hidden = items.length !== 0;
  $('#empty-title').textContent = state.view === 'saved' && !saved.length ? 'Your next movie night starts here' : 'No titles found';
  $('#empty-description').textContent = state.view === 'saved' && !saved.length ? 'Tap + on any title to build your personal list.' : 'Try a different search, content type, or genre.';
  $('#reset-button').textContent = state.view === 'saved' && !saved.length ? 'Explore titles' : 'Reset filters';
  $('#load-more').hidden = count >= items.length;
  updateSaveButtons();
}
function updateGenres() {
  const genres = [...new Set(state.items.flatMap(item => item.genres))].sort();
  if (!genres.includes(state.genre)) state.genre = '';
  const option = (value, text) => { const node = el('option', '', text); node.value = value; return node; };
  $('#genre').replaceChildren(option('', 'All genres'), ...genres.map(genre => option(genre, genre)));
  $('#genre').value = state.genre;
}
function updateHeading() {
  const titles = { discover: 'Popular right now', movie: 'Movies to get lost in', series: 'Your next binge', saved: 'My list' };
  $('#catalog-title').textContent = state.query ? `Results for “${state.query}”` : state.sort === 'imdbRating' && state.view !== 'saved' ? 'The highest-rated stories' : titles[state.view];
  $('#section-eyebrow').textContent = state.view === 'saved' ? 'SAVED FOR A GOOD NIGHT' : state.query ? 'FIND YOUR NEXT FAVORITE' : 'A LITTLE INSPIRATION';
  $('#catalog-subtitle').textContent = state.view === 'saved' ? 'Your personal collection, saved in this browser.' : 'Live from Cinemeta. Something great is waiting.';
  $('#hero').hidden = state.view !== 'discover' || Boolean(state.query);
  $('#catalog-sort').disabled = Boolean(state.query) || state.view === 'saved';
  document.querySelectorAll('[data-view]').forEach(button => {
    const active = button.dataset.view === state.view;
    button.classList.toggle('active', active);
    if (active) button.setAttribute('aria-current', 'page'); else button.removeAttribute('aria-current');
  });
  document.querySelectorAll('[data-type]').forEach(button => {
    const active = button.dataset.type === state.type;
    button.classList.toggle('active', active);
    button.setAttribute('aria-pressed', String(active));
  });
}
function showNotice(message) {
  $('#notice').textContent = message;
  $('#notice').hidden = !message;
}
function setSpotlight(item) {
  spotlight = item;
  $('#hero-title').textContent = item.title;
  $('#hero-meta').textContent = [rating(item) ? `★ ${rating(item)} IMDb` : '', item.year, item.media_type === 'series' ? 'TV show' : 'Movie', ...item.genres.slice(0, 2)].filter(Boolean).join('   ·   ');
  $('#hero-description').textContent = item.description || 'A new story is waiting. Explore the details and save this title for your next movie night.';
  const image = $('#hero-image');
  image.hidden = true;
  image.onload = () => { image.hidden = false; };
  image.onerror = () => { image.hidden = true; };
  if (item.background) image.src = safeImage(item.background); else image.removeAttribute('src');
  $('#hero-details').disabled = false;
  $('#hero-save').disabled = false;
  configureSave($('#hero-save'), item);
  updateSaveButtons();
}
async function loadCatalog() {
  catalogController?.abort();
  catalogController = new AbortController();
  const signal = catalogController.signal;
  const current = ++generation;
  state.visible = 18;
  state.genre = '';
  updateHeading();
  showNotice('');
  $('#retry').hidden = true;
  $('#empty').hidden = true;
  $('#load-more').hidden = true;
  if (state.view === 'saved') {
    state.loading = false;
    state.items = saved;
    updateGenres();
    renderGrid();
    return;
  }
  state.loading = true;
  $('#grid').setAttribute('aria-busy', 'true');
  $('#results-status').textContent = state.query ? 'Searching Cinemeta…' : 'Finding great stories…';
  $('#genre').disabled = true;
  showSkeletons();
  const types = state.type === 'all' ? ['movie', 'series'] : [state.type];
  const responses = await Promise.allSettled(types.map(type => fetchCatalog(type, state.sort, state.query, signal)));
  if (current !== generation || signal.aborted) return;
  const successful = responses.filter(result => result.status === 'fulfilled');
  const errors = responses.filter(result => result.status === 'rejected');
  const lists = successful.map(result => result.value);
  // Interleave movies and series rather than burying an entire type below the fold.
  const combined = [];
  const seen = new Set();
  for (let i = 0; i < Math.max(0, ...lists.map(items => items.length)); i++) {
    for (const items of lists) {
      const item = items[i];
      if (item && !seen.has(key(item))) { seen.add(key(item)); combined.push(item); }
    }
  }
  state.items = combined;
  state.loading = false;
  $('#genre').disabled = false;
  updateGenres();
  renderGrid();
  if (errors.length) {
    showNotice(successful.length ? 'One catalog could not be loaded. Available titles are shown below; retry to load both catalogs.' : errors[0].reason.message || 'The catalog is temporarily unavailable.');
    $('#retry').hidden = false;
    if (!successful.length) {
      $('#empty').hidden = true;
      $('#results-status').textContent = 'Unable to load the catalog.';
    }
  }
  if (!state.query && state.view === 'discover' && combined.length) {
    const item = combined[0];
    setSpotlight(item);
    try {
      const detail = await fetchDetails(item, signal);
      if (current === generation && !signal.aborted) setSpotlight(detail);
    } catch { /* Catalog information remains usable if spotlight enrichment fails. */ }
  }
}
function renderDetails(item, status = '', failed = false) {
  const body = $('#detail-body');
  body.replaceChildren();
  const layout = el('div', 'detail-layout');
  const content = el('div', 'detail-content');
  const title = el('h2', '', item.title);
  title.id = 'detail-title';
  content.append(title, el('div', 'detail-meta', [item.year, item.media_type === 'series' ? 'TV show' : 'Movie', item.duration, rating(item) ? `★ ${rating(item)} IMDb` : ''].filter(Boolean).join(' · ')));
  const genres = el('div', 'detail-genres');
  genres.append(...item.genres.map(genre => el('span', '', genre)));
  content.append(genres, el('p', 'detail-description', item.description || 'No synopsis is available for this title yet.'));
  const credits = el('dl', 'credits');
  if (item.director) credits.append(el('dt', '', 'Director'), el('dd', '', item.director));
  if (item.stars) credits.append(el('dt', '', 'Cast'), el('dd', '', item.stars));
  content.append(credits);
  const actions = el('div', 'detail-actions');
  const save = el('button', 'primary');
  configureSave(save, item);
  actions.append(save);
  content.append(actions);
  layout.append(poster(item, 'detail-poster'), content);
  body.append(layout);
  if (status) {
    const statusBox = el('div', 'detail-loading', status);
    statusBox.setAttribute('role', 'status');
    if (failed) {
      const retry = el('button', 'secondary', 'Retry details');
      retry.style.marginLeft = '12px';
      retry.onclick = () => openDetails(item);
      statusBox.append(retry);
    }
    body.append(statusBox);
  }
  if (item.seasons.length) {
    const section = el('section', 'episode-section');
    const heading = el('div', 'episode-heading');
    const select = el('select');
    select.setAttribute('aria-label', 'Select season');
    item.seasons.forEach((season, index) => { const option = el('option', '', season.number === 0 ? 'Specials' : `Season ${season.number}`); option.value = index; select.append(option); });
    const list = el('ol', 'episode-list');
    const renderEpisodes = () => {
      list.replaceChildren(...item.seasons[Number(select.value)].episodes.map(episode => {
        const row = el('li');
        const text = el('div');
        text.append(el('strong', '', episode.title || `Episode ${episode.number}`));
        if (episode.overview) text.append(el('p', '', episode.overview));
        row.append(el('span', 'episode-number', String(episode.number).padStart(2, '0')), text);
        return row;
      }));
    };
    heading.append(el('h3', '', 'Episodes'), select);
    select.onchange = renderEpisodes;
    renderEpisodes();
    section.append(heading, list);
    body.append(section);
  }
  const note = el('div', 'detail-note');
  note.append(document.createTextNode('Catalog information only — Cinemeta does not provide video streams. For native playback and supported streaming providers, '));
  const link = el('a', '', 'get MovieBox-TUI ↗');
  link.href = 'https://github.com/dyan80514-svg/MovieBox-Tui#installation';
  link.target = '_blank';
  link.rel = 'noopener noreferrer';
  note.append(link);
  body.append(note);
  updateSaveButtons();
}
async function openDetails(item) {
  detailController?.abort();
  detailController = new AbortController();
  const signal = detailController.signal;
  selected = item;
  renderDetails(item, 'Loading full details…');
  const dialog = $('#details-dialog');
  if (!dialog.open) dialog.showModal();
  dialog.scrollTop = 0;
  try {
    const detail = await fetchDetails(item, signal);
    if (signal.aborted || !dialog.open) return;
    selected = detail;
    renderDetails(detail);
  } catch (error) {
    if (signal.aborted || !dialog.open) return;
    renderDetails(selected, error.message || 'Unable to load full details.', true);
  }
}
function navigate(view) {
  state.view = view;
  state.type = ['movie', 'series'].includes(view) ? view : 'all';
  state.query = '';
  $('#search').value = '';
  loadCatalog();
}
document.querySelectorAll('[data-view]').forEach(button => { button.onclick = () => navigate(button.dataset.view); });
document.querySelectorAll('[data-type]').forEach(button => {
  button.onclick = () => {
    state.type = button.dataset.type;
    if (state.view !== 'saved') state.view = state.type === 'all' ? 'discover' : state.type;
    loadCatalog();
  };
});
$('#search-form').onsubmit = event => {
  event.preventDefault();
  state.query = $('#search').value.trim();
  loadCatalog();
};
$('#search').addEventListener('input', () => {
  if (!$('#search').value && state.query) { state.query = ''; loadCatalog(); }
});
$('#catalog-sort').onchange = event => { state.sort = event.target.value; loadCatalog(); };
$('#genre').onchange = event => { state.genre = event.target.value; state.visible = 18; renderGrid(); };
$('#load-more').onclick = () => { state.visible += 18; renderGrid(); };
$('#retry').onclick = loadCatalog;
$('#reset-button').onclick = () => {
  if (state.view === 'saved' && !saved.length) { navigate('discover'); return; }
  state.query = ''; state.type = 'all'; state.genre = '';
  if (state.view !== 'saved') state.view = 'discover';
  $('#search').value = '';
  loadCatalog();
};
$('#hero-details').onclick = () => { if (spotlight) openDetails(spotlight); };
$('#close-details').onclick = () => $('#details-dialog').close();
$('#details-dialog').addEventListener('close', () => detailController?.abort());
$('#about-button').onclick = () => $('#about-dialog').showModal();
$('#close-about').onclick = () => $('#about-dialog').close();
for (const dialog of document.querySelectorAll('dialog')) {
  dialog.addEventListener('click', event => {
    const rect = dialog.getBoundingClientRect();
    if (event.target === dialog && (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom)) dialog.close();
  });
}
document.addEventListener('keydown', event => {
  if (event.key === '/' && !event.ctrlKey && !event.metaKey && !event.altKey && !document.querySelector('dialog[open]') && !['INPUT', 'TEXTAREA', 'SELECT'].includes(document.activeElement.tagName)) {
    event.preventDefault();
    $('#search').focus();
  }
});
window.addEventListener('storage', event => {
  if (event.key === STORAGE_KEY || event.key === null) {
    saved = readSaved();
    updateSaveButtons();
    if (state.view === 'saved') { state.items = saved; updateGenres(); renderGrid(); }
  }
});
updateSaveButtons();
loadCatalog();
