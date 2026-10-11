// Gamercards: /card/<gamertag>.svg (drawn live) and /card/<gamertag>.png
// (the owner's browser renders the SVG and uploads it; see site.js).

import { cardSvg } from '../_card.js';

const GAMES_DB = 'https://raw.githubusercontent.com/IKryptikI/achievement-hub-games/main/site/';

function b64(buf) {
  const bytes = new Uint8Array(buf);
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

async function dataUri(fetcher, url) {
  try {
    const r = await fetcher(url);
    if (!r.ok) return null;
    const type = r.headers.get('content-type') || 'image/png';
    if (!type.startsWith('image/')) return null;
    return `data:${type};base64,${b64(await r.arrayBuffer())}`;
  } catch {
    return null;
  }
}

/** Icons for the latest unlocks, looked up in the game database. */
async function recentIcons(snapshot) {
  const games = snapshot.games || [];
  const picks = (snapshot.recent || []).slice(0, 5);
  const byGame = {};
  for (const r of picks) {
    const gid = r.game || (games.find((g) => g.title === r.title) || {}).id;
    if (gid) (byGame[gid] ||= []).push(r);
  }
  const icons = new Map();
  await Promise.all(Object.keys(byGame).map(async (gid) => {
    try {
      const d = await (await fetch(GAMES_DB + encodeURIComponent(gid) + '.json', { cf: { cacheTtl: 3600 } })).json();
      for (const r of byGame[gid]) {
        const a = d.achievements.find((x) => (r.id ? x.id === r.id : x.name === r.name));
        if (a && a.icon) icons.set(r, a.icon);
      }
    } catch {}
  }));
  const uris = await Promise.all(picks.map((r) => (icons.has(r) ? dataUri((u) => fetch(u, { cf: { cacheTtl: 86400 } }), icons.get(r)) : null)));
  return uris.filter(Boolean).map((uri) => ({ uri }));
}

export async function onRequestGet({ request, env, params }) {
  const m = String(params.name || '').match(/^(.+)\.(svg|png)$/);
  if (!m) return new Response('Not found', { status: 404 });
  const tag = decodeURIComponent(m[1]);
  const u = await env.DB.prepare('SELECT id, gamertag, motto, color, avatar, private FROM users WHERE gamertag = ?').bind(tag).first();
  if (!u) return new Response('No such gamertag', { status: 404 });

  if (m[2] === 'png') {
    const row = await env.DB.prepare('SELECT png, updated FROM gamercards WHERE user_id = ?').bind(u.id).first();
    if (!row) return env.ASSETS.fetch(new URL('/assets/og.png', request.url));
    return new Response(new Uint8Array(row.png), {
      headers: { 'content-type': 'image/png', 'cache-control': 'public, max-age=600', 'last-modified': new Date(row.updated * 1000).toUTCString() },
    });
  }

  const progress = await env.DB.prepare('SELECT snapshot, gamerscore FROM progress WHERE user_id = ?').bind(u.id).first();
  const snapshot = progress ? JSON.parse(progress.snapshot) : {};
  const asset = (path) => env.ASSETS.fetch(new URL(path, request.url));
  const own = u.avatar.startsWith('img:') ? await env.DB.prepare('SELECT img, type FROM gamerpics WHERE user_id = ?').bind(u.id).first() : null;
  const pic = own ? `data:${own.type};base64,${b64(own.img)}` : null;
  const icons = u.private ? [] : await recentIcons(snapshot);
  const fonts = await Promise.all(['selawk', 'selawksb'].map(async (f) => b64(await (await asset(`/assets/${f}.ttf`)).arrayBuffer())));
  let svg = cardSvg(
    { gamertag: u.gamertag, motto: u.motto, color: u.color, gamerscore: progress ? progress.gamerscore : 0, unlocked: snapshot.unlocked || 0, avatarLetter: u.avatar.startsWith('img:') ? u.gamertag[0].toUpperCase() : u.avatar },
    pic,
    icons,
  );
  // Fonts inside the SVG, so it looks the same wherever it's drawn.
  svg = svg.replace('<defs>', `<defs><style>@font-face{font-family:Selawik;font-weight:400;src:url(data:font/ttf;base64,${fonts[0]})}@font-face{font-family:Selawik;font-weight:600;src:url(data:font/ttf;base64,${fonts[1]})}</style>`);
  return new Response(svg, { headers: { 'content-type': 'image/svg+xml', 'cache-control': 'public, max-age=300' } });
}
