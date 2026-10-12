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

/** The gamer picture: the player's uploaded one, or a letter/emoji on their color. */
function pic(p, small = false) {
  const a = p.avatar || '';
  if (a.startsWith('img:') && p.gamertag) return `<div class="pic img${small ? ' small' : ''}"><img src="/pic/${encodeURIComponent(p.gamertag)}?v=${esc(a.slice(4))}" alt="" loading="lazy"></div>`;
  return `<div class="pic${small ? ' small' : ''}" style="--accent:${esc(p.color || '#4cc931')}">${esc(a && !a.startsWith('img:') ? a : (p.gamertag || '?')[0].toUpperCase())}</div>`;
}

/** Crops an image file to a square and shrinks it to a 256px JPEG. */
async function squarePicture(file) {
  const url = URL.createObjectURL(file);
  try {
    const img = new Image();
    await new Promise((ok, bad) => { img.onload = ok; img.onerror = () => bad(new Error("That file isn't a picture we can read.")); img.src = url; });
    const side = Math.min(img.naturalWidth, img.naturalHeight);
    const c = document.createElement('canvas');
    c.width = c.height = 256;
    const g = c.getContext('2d');
    g.fillStyle = '#ffffff';
    g.fillRect(0, 0, 256, 256);
    g.drawImage(img, (img.naturalWidth - side) / 2, (img.naturalHeight - side) / 2, side, side, 0, 0, 256, 256);
    return await new Promise((ok) => c.toBlob(ok, 'image/jpeg', 0.88));
  } finally {
    URL.revokeObjectURL(url);
  }
}
const ago = (t) => {
  const s = Date.now() / 1000 - t;
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  if (s < 86400 * 30) return `${Math.floor(s / 86400)}d ago`;
  return new Date(t * 1000).toLocaleDateString();
};
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
    ['index.html', 'Home'], ['games.html', 'Games'], ['forum.html', 'Forum'], ['requests.html', 'Requests'], ['friends.html', 'Friends'],
    ['download.html', 'Download'], ['about.html', 'About'],
  ];
  const active = { game: 'games', thread: 'forum', bounties: 'games' }[here] || here;
  document.body.insertAdjacentHTML('afterbegin', `
    <header class="top"><div class="wrap">
      <a class="brand" href="index.html"><span class="orb"></span>Achievement Hub</a>
      <nav class="main">${links.map(([h, t]) => `<a href="${h}"${!h.startsWith('http') && page(h) === active ? ' class="on"' : ''}${h.startsWith('http') ? ' target="_blank" rel="noopener"' : ''}>${t}</a>`).join('')}</nav>
      <button class="theme-btn" id="theme" title="Light / dark"></button>
      <div class="account" id="account"></div>
      <button class="menu-btn" id="menu" aria-label="Menu" aria-expanded="false"><i></i><i></i><i></i></button>
    </div></header>`);
  setTheme(document.documentElement.dataset.theme || 'dark');
  // Phones: the links fold into a menu.
  $('#menu').onclick = () => {
    const open = document.querySelector('header.top').classList.toggle('open');
    $('#menu').setAttribute('aria-expanded', open);
  };
  $('#theme').onclick = () => setTheme(document.documentElement.dataset.theme === 'light' ? 'dark' : 'light');
  document.querySelector('main')?.classList.add('page-in');
  // Primary buttons get a light sweep.
  const sweep = () => document.querySelectorAll('.btn.primary:not([data-sw])').forEach((b) => { b.dataset.sw = 1; b.insertAdjacentHTML('beforeend', '<span class="sweep"></span>'); });
  sweep();
  new MutationObserver(sweep).observe(document.body, { childList: true, subtree: true });
  document.body.insertAdjacentHTML('beforeend', `
    <footer><div class="wrap">
      <span>Achievement Hub: achievements for the PC games that never had them. Free and open source (GPL-3.0).</span>
      <span><a href="news.html">News</a> · <a href="mappers.html">Become a mapper</a> · <a href="bounties.html">Mapping board</a> · <a href="faq.html">FAQ</a> · <a href="privacy.html">Privacy</a> · <a href="terms.html">Terms</a> · <a href="about.html#credits">Credits</a> · <a href="${SITE.github}" target="_blank" rel="noopener">GitHub</a></span>
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
  // (The same account links inside the phone menu.)
  $('nav.main').insertAdjacentHTML('beforeend', m
    ? `<a class="m-only" href="account.html">Account</a>${m.is_admin ? '<a class="m-only" href="admin.html">Admin</a>' : ''}`
    : '<a class="m-only" href="login.html">Log in</a><a class="m-only" href="signup.html">Sign up</a>');
  if (m) {
    emailBanner(m);
    siteUnlocks(m);
    // Keep the gamercard image fresh (drawn here, see refreshCard).
    if (Date.now() / 1000 - (m.card_updated || 0) > 3 * 3600 || (m.sent || 0) > (m.card_updated || 0))
      setTimeout(() => refreshCard(m).catch(() => {}), 1500);
  }
})();

/** Draws /card/<you>.svg to a PNG and saves it, so links to your
 * gamercard work in Discord and forums (which don't show SVG). */
async function refreshCard(m) {
  const svg = await (await fetch(`/card/${encodeURIComponent(m.gamertag)}.svg?t=${Date.now()}`)).text();
  const url = URL.createObjectURL(new Blob([svg], { type: 'image/svg+xml' }));
  try {
    const img = new Image();
    await new Promise((ok, bad) => { img.onload = ok; img.onerror = bad; img.src = url; });
    const c = document.createElement('canvas');
    c.width = 1200; c.height = 420;
    c.getContext('2d').drawImage(img, 0, 0, c.width, c.height);
    const png = await new Promise((ok) => c.toBlob(ok, 'image/png'));
    await fetch('/api/me/card', { method: 'PUT', body: png, credentials: 'same-origin' });
  } finally {
    URL.revokeObjectURL(url);
  }
}

/** Site achievements earned since this browser last looked pop up like in-game ones. */
function siteUnlocks(m) {
  const key = 'ah-site-ach-' + m.gamertag;
  let seen = null;
  try { seen = JSON.parse(localStorage.getItem(key)); } catch {}
  const have = (m.achievements || []).filter((a) => a.unlocked).map((a) => a.id);
  try { localStorage.setItem(key, JSON.stringify(have)); } catch {}
  if (!Array.isArray(seen)) seen = have.filter((id) => id === 'founder' || id === 'dev'); // first visit: pop the rest once
  const fresh = (m.achievements || []).filter((a) => a.unlocked && !seen.includes(a.id)).slice(0, 4);
  fresh.forEach((a, i) => setTimeout(() => siteToast(a), 900 + i * 4200));
}
function siteToast(a) {
  const el = document.createElement('div');
  el.className = 'site-toast';
  el.innerHTML = `<div class="ring">${a.icon}</div><div><small>Site achievement unlocked</small><b>${esc(a.name)}</b></div>`;
  document.body.append(el);
  requestAnimationFrame(() => el.classList.add('on'));
  setTimeout(() => { el.classList.remove('on'); setTimeout(() => el.remove(), 600); }, 3600);
}

// ── Forum text: plain text with links, pictures, **bold** and `code` ──
function richText(s) {
  return esc(s)
    .replace(/`([^`\n]+)`/g, '<code>$1</code>')
    .replace(/\*\*([^*\n]+)\*\*/g, '<b>$1</b>')
    .replace(/https:\/\/[^\s<]+/g, (u) => /\.(png|jpe?g|gif|webp)(\?\S*)?$/i.test(u)
      ? `<a href="${u}" target="_blank" rel="noopener nofollow"><img class="post-img" src="${u}" alt="" loading="lazy"></a>`
      : `<a href="${u}" target="_blank" rel="noopener nofollow ugc">${u}</a>`)
    .replace(/\n/g, '<br>');
}
const TAG_LOOK = {
  announcements: ['📌', 'Announcements'], help: ['🙋', 'Help'], bug: ['🐞', 'Bug'], 'game-support': ['🎮', 'Game support'],
  mapping: ['🗺️', 'Mapping'], suggestion: ['💡', 'Suggestion'], showcase: ['🏆', 'Showcase'], general: ['💬', 'General'],
};
const tagChip = (t) => `<span class="ftag ftag-${esc(t)}">${(TAG_LOOK[t] || ['', t])[1]}</span>`;
const nameTag = (x) => `<span class="fname" style="color:${esc(x.color || '#4cc931')}">${esc(x.gamertag || 'deleted')}</span>${x.is_admin ? '<span class="dev-tag">DEV</span>' : ''}`;

// ── Protection: site settings, the "I'm human" check, reports ──
let CONFIG = null;
const siteConfig = () => (CONFIG ||= fetch('/api/config').then((r) => r.json()).catch(() => ({ turnstile: null, email: false })));

/** Puts Cloudflare's bot check in `el` (if it's set up). Returns a function
 * giving the token to send along (empty if not set up), and a reset. */
async function humanCheck(el) {
  const cfg = await siteConfig();
  if (!cfg.turnstile) return { token: () => '', reset: () => {} };
  if (!window.turnstile) {
    await new Promise((ok) => {
      const s = document.createElement('script');
      s.src = 'https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit';
      s.onload = ok;
      document.head.append(s);
    });
  }
  let token = '';
  const id = window.turnstile.render(el, {
    sitekey: cfg.turnstile,
    theme: document.documentElement.dataset.theme === 'light' ? 'light' : 'dark',
    callback: (t) => (token = t),
    'expired-callback': () => (token = ''),
  });
  return { token: () => token, reset: () => { token = ''; window.turnstile.reset(id); } };
}

/** Asks why, then reports something to the admin. */
async function report(kind, target) {
  if (!(await me())) return (location.href = 'login.html?next=' + encodeURIComponent(location.pathname + location.search));
  const reason = prompt('What is wrong with it? (spam, slurs, harassment, something else)');
  if (reason === null) return;
  try { await api('/reports', { method: 'POST', body: { kind, target: String(target), reason } }); alert('Thanks, the admin will take a look.'); }
  catch (err) { alert(err.message); }
}

/** A banner asking to add or verify an email, when that's needed. */
async function emailBanner(m) {
  const cfg = await siteConfig();
  if (!m || m.is_admin || (m.email && (m.email_verified || !cfg.email))) return;
  const el = document.createElement('div');
  el.className = 'notice-bar';
  if (!m.email) {
    el.innerHTML = `<div class="wrap">Add an email address to your account (needed to post and to reset your password). <a href="account.html#email">Add it</a></div>`;
  } else {
    el.innerHTML = `<div class="wrap"><span>We sent a 6-digit code to <b>${esc(m.email)}</b>. Enter it to verify your email:</span>
      <input id="vcode" inputmode="numeric" maxlength="7" placeholder="123456" style="width:110px">
      <button class="btn small primary" id="vgo">Verify</button><button class="linkbtn" id="vre">Send a new code</button><span id="vmsg"></span></div>`;
  }
  document.querySelector('header.top')?.after(el);
  $('#vgo')?.addEventListener('click', async () => {
    try { await api('/me/verify', { method: 'POST', body: { code: $('#vcode').value } }); el.innerHTML = '<div class="wrap">Email verified. Thanks!</div>'; setTimeout(() => el.remove(), 2500); }
    catch (err) { $('#vmsg').textContent = err.message; }
  });
  $('#vre')?.addEventListener('click', async () => {
    try { await api('/me/verify/resend', { method: 'POST' }); $('#vmsg').textContent = 'Sent!'; } catch (err) { $('#vmsg').textContent = err.message; }
  });
}
