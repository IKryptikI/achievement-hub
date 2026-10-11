// Link previews for shared game and profile pages: fills in the page's
// og:title / og:description / og:image for that game or player, so a link
// pasted into Discord shows the game's cover or the player's Gamerscore.

const GAMES_DB = 'https://raw.githubusercontent.com/IKryptikI/achievement-hub-games/main/site/';

function setMeta(res, meta) {
  const rw = new HTMLRewriter();
  for (const [prop, value] of Object.entries(meta)) {
    if (!value) continue;
    rw.on(`meta[property="${prop}"]`, { element: (el) => el.setAttribute('content', value) });
  }
  if (meta['og:title']) rw.on('title', { element: (el) => el.setInnerContent(meta['og:title']) });
  return rw.transform(res);
}

export async function onRequest({ request, next, env }) {
  const url = new URL(request.url);
  const page = url.pathname.replace(/\.html$/, '');
  const res = await next();
  if (request.method !== 'GET' || !(res.headers.get('content-type') || '').includes('text/html')) return res;
  try {
    if (page === '/game' && url.searchParams.get('g')) {
      const g = await (await fetch(GAMES_DB + encodeURIComponent(url.searchParams.get('g')) + '.json', { cf: { cacheTtl: 600 } })).json();
      return setMeta(res, {
        'og:title': `${g.title} achievements · Achievement Hub`,
        'og:description': `${g.achievements.length} achievements, ${g.points} Gamerscore. See which ones are confirmed and earn them on PC with Achievement Hub.`,
        'og:image': g.cover || g.banner,
      });
    }
    if (page === '/profile' && url.searchParams.get('u')) {
      const u = await env.DB.prepare(
        'SELECT u.gamertag, u.motto, u.private, p.gamerscore FROM users u LEFT JOIN progress p ON p.user_id = u.id WHERE u.gamertag = ?',
      ).bind(url.searchParams.get('u')).first();
      if (u) {
        return setMeta(res, {
          'og:title': `${u.gamertag} · Achievement Hub`,
          'og:description': `${u.gamerscore || 0} Gamerscore${u.motto ? ' · ' + u.motto : ''}`,
          'og:image': `${url.origin}/card/${encodeURIComponent(u.gamertag)}.png`,
        });
      }
    }
  } catch {}
  return res;
}
