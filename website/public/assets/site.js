// Shared bits: header, footer, side banners, background, scroll reveals,
// the API helper, login state and the game database.
const SITE = {
  github: 'https://github.com/IKryptikI/achievement-hub',
  gamesDb: 'https://raw.githubusercontent.com/IKryptikI/achievement-hub-games/main',
  releases: 'https://github.com/IKryptikI/achievement-hub/releases/latest',
};

async function api(path, options = {}) {
  const res = await fetch('/api' + path, {
    credentials: 'same-origin',
    headers: options.body ? { 'content-type': 'application/json' } : {},
    ...options,
    body: options.body ? JSON.stringify(options.body) : undefined,
  });
  let data = {};
  try { data = await res.json(); } catch {}
  if (!res.ok) throw new Error(data.error || `Something went wrong (${res.status}).`);
  return data;
}

let ME = null;
async function me() {
  if (ME !== null) return ME;
  try { ME = await api('/me'); } catch { ME = false; }
  return ME;
}

const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
const $ = (sel, root = document) => root.querySelector(sel);

function pic(p, small = false) {
  return `<div class="pic${small ? ' small' : ''}" style="--accent:${esc(p.color || '#4cc931')}">${esc(p.avatar || (p.gamertag || '?')[0].toUpperCase())}</div>`;
}
const online = (p) => p.sent && Date.now() / 1000 - p.sent < 15 * 60;
function note(el, text, good) {
  el.innerHTML = text ? `<div class="note ${good ? 'good' : 'bad'}">${esc(text)}</div>` : '';
}

// ── The game database (published with the game files) ──
let GAMES = null;
async function games() {
  if (!GAMES) GAMES = fetch(SITE.gamesDb + '/site/games.json').then((r) => r.json()).then((d) => d.games);
  return GAMES;
}
async function game(id) {
  return (await fetch(SITE.gamesDb + `/site/${encodeURIComponent(id)}.json`)).json();
}
function coverCard(g, extra = '') {
  const pct = g.total ? Math.round((100 * g.tracked) / g.total) : 0;
  return `<a class="cover" href="game.html?g=${encodeURIComponent(g.id)}" title="${esc(g.title)}">
    ${g.cover ? `<img src="${esc(g.cover)}" alt="" loading="lazy">` : `<div class="noart">${esc(g.title)}</div>`}
    <span class="chip dark">${g.total} · ${pct}% mapped</span>
    <div class="info"><b>${esc(g.title)}</b><span>${g.points} G${g.year ? ' · ' + g.year : ''}</span>${extra}</div></a>`;
}

// ── Password helpers (show/hide, strength) ──
function passwordField(input) {
  const wrap = document.createElement('div');
  wrap.className = 'pw';
  input.replaceWith(wrap);
  wrap.append(input);
  const b = document.createElement('button');
  b.type = 'button';
  b.textContent = 'Show';
  b.onclick = () => { const s = input.type === 'password'; input.type = s ? 'text' : 'password'; b.textContent = s ? 'Hide' : 'Show'; };
  wrap.append(b);
}
function strengthMeter(input) {
  const m = document.createElement('div');
  m.className = 'meter';
  m.innerHTML = '<i></i>';
  const label = document.createElement('div');
  label.className = 'muted';
  label.style.fontSize = '.85rem';
  (input.closest('.pw') || input).after(m, label);
  input.addEventListener('input', () => {
    const v = input.value;
    let s = Math.min(v.length, 16) / 4;
    for (const re of [/[a-z]/, /[A-Z]/, /\d/, /[^A-Za-z0-9]/]) if (re.test(v)) s += 1;
    const lvl = v.length < 8 ? 0 : s < 5 ? 1 : s < 7 ? 2 : 3;
    m.firstChild.style.width = (v ? [20, 45, 75, 100][lvl] : 0) + '%';
    m.firstChild.style.background = ['#ef5d55', '#f0b44c', '#7cf25c', '#2bd3c4'][lvl];
    label.textContent = v ? ['Too short (8+ characters)', 'Okay', 'Good', 'Strong'][lvl] : '';
  });
}

// ── Loader: shown when leaving for another page of the site ──
function loader(on) {
  let el = document.getElementById('loader');
  if (!el) {
    document.body.insertAdjacentHTML('beforeend', '<div id="loader" aria-hidden="true"><div class="spin"><i></i><i></i><i></i><b></b></div></div>');
    el = document.getElementById('loader');
  }
  el.classList.toggle('on', on);
}
document.addEventListener('click', (e) => {
  const a = e.target.closest('a[href]');
  if (!a || e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || a.target === '_blank') return;
  const url = new URL(a.href, location.href);
  if (url.origin !== location.origin || (url.pathname === location.pathname && url.search === location.search)) return;
  loader(true);
});
window.addEventListener('pageshow', () => loader(false));

/** Placeholder shapes while data loads. */
function skeleton(kind, n) {
  const one = kind === 'cover' ? '<div class="skel" style="aspect-ratio:2/3"></div>' : '<div class="skel" style="height:92px;margin-bottom:10px"></div>';
  return one.repeat(n);
}

// ── Light/dark theme (remembered per browser) ──
function setTheme(t) {
  document.documentElement.dataset.theme = t;
  try { localStorage.setItem('ah-theme', t); } catch {}
  const b = document.getElementById('theme');
  if (b) b.textContent = t === 'light' ? '☾' : '☀';
}

// ── Page chrome ──
(async function chrome() {
  document.body.insertAdjacentHTML('afterbegin', '<div class="aurora"><i></i><i></i><i></i></div><div class="gridbg"></div>');
  // (Cloudflare serves pages without ".html": /games for games.html.)
  const page = (p) => p.replace(/\.html$/, '').replace(/^index$/, '') || '';
  const here = page(location.pathname.split('/').pop());
  const links = [
    ['index.html', 'Home'], ['games.html', 'Games'], ['requests.html', 'Requests'], ['friends.html', 'Friends'],
    ['download.html', 'Download'], ['about.html', 'About'],
  ];
  const active = here === 'game' ? 'games' : here;
  document.body.insertAdjacentHTML('afterbegin', `
    <header class="top"><div class="wrap">
      <a class="brand" href="index.html"><span class="orb"></span>Achievement Hub</a>
      <nav class="main">${links.map(([h, t]) => `<a href="${h}"${!h.startsWith('http') && page(h) === active ? ' class="on"' : ''}${h.startsWith('http') ? ' target="_blank" rel="noopener"' : ''}>${t}</a>`).join('')}</nav>
      <button class="theme-btn" id="theme" title="Light / dark"></button>
      <div class="account" id="account"></div>
    </div></header>`);
  setTheme(document.documentElement.dataset.theme || 'dark');
  $('#theme').onclick = () => setTheme(document.documentElement.dataset.theme === 'light' ? 'dark' : 'light');
  document.querySelector('main')?.classList.add('page-in');
  // Primary buttons get a light sweep.
  const sweep = () => document.querySelectorAll('.btn.primary:not([data-sw])').forEach((b) => { b.dataset.sw = 1; b.insertAdjacentHTML('beforeend', '<span class="sweep"></span>'); });
  sweep();
  new MutationObserver(sweep).observe(document.body, { childList: true, subtree: true });
  document.body.insertAdjacentHTML('beforeend', `
    <footer><div class="wrap">
      <span>Achievement Hub: achievements for the PC games that never had them. Free and open source (GPL-3.0).</span>
      <span><a href="news.html">News</a> · <a href="mappers.html">Become a mapper</a> · <a href="faq.html">FAQ</a> · <a href="privacy.html">Privacy</a> · <a href="terms.html">Terms</a> · <a href="about.html#credits">Credits</a> · <a href="${SITE.github}" target="_blank" rel="noopener">GitHub</a></span>
    </div></footer>`);

  // Reveal sections as they scroll in.
  const io = new IntersectionObserver((es) => es.forEach((e) => e.isIntersecting && (e.target.classList.add('in'), io.unobserve(e.target))), { threshold: 0.12 });
  const watch = () => document.querySelectorAll('.reveal:not(.in)').forEach((el) => io.observe(el));
  watch();
  new MutationObserver(watch).observe(document.body, { childList: true, subtree: true });

  // Side banners: every supported game's cover, scrolling forever.
  games().then((list) => {
    const art = list.filter((g) => g.cover).map((g) => g.cover);
    if (!art.length) return;
    for (const side of ['left', 'right']) {
      const order = side === 'left' ? art : [...art].reverse();
      const many = [];
      while (many.length < 10) many.push(...order);
      const imgs = [...many, ...many].map((u) => `<img src="${esc(u)}" alt="" loading="lazy">`).join('');
      document.body.insertAdjacentHTML('afterbegin', `<div class="rail ${side}" aria-hidden="true"><div class="col">${imgs}</div></div>`);
    }
  }).catch(() => {});

  const m = await me();
  $('#account').innerHTML = m
    ? `<a href="profile.html?u=${encodeURIComponent(m.gamertag)}" class="person" title="Your profile">${pic(m, true)}</a>
       <a class="btn small" href="account.html">Account</a>${m.is_admin ? '<a class="btn small" href="admin.html">Admin</a>' : ''}`
    : `<a class="btn small" href="login.html">Log in</a><a class="btn small primary" href="signup.html">Sign up</a>`;
})();
