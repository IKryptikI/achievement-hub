// Community features for the API ([[route]].js calls this before giving up):
//
// Mappers:   POST /api/mappers {name, email, discord, platform, games, experience, hours, notes}
// Requests:  GET /api/requests   POST /api/requests {title, store, notes}   POST /api/requests/:id/vote
// Guides:    GET /api/games/:id/guides   POST /api/games/:id/guides {achievement, body}
//            POST /api/guides/:id/vote   DELETE /api/guides/:id
// Boards:    GET /api/games/:id/leaderboard
// Feed:      GET /api/feed                       (friends' recent unlocks)
// Compare:   GET /api/compare/:gamertag/:game    (your unlocks and theirs)
// News:      GET /api/news
// Admin:     GET /api/admin/summary   PATCH /api/admin/mappers/:id {status}
//            PATCH /api/admin/requests/:id {status}   POST /api/admin/news   DELETE /api/admin/news/:id

const STATUSES = { mapper: ['new', 'contacted', 'accepted', 'declined'], request: ['open', 'planned', 'added', 'declined'] };
const clip = (s, n) => String(s ?? '').trim().slice(0, n);

/** At most `max` calls per `window` seconds for this key (per connection). */
export async function rateLimit(env, req, action, max, window) {
  const ip = req.headers.get('cf-connecting-ip') || 'local';
  const key = `${action}:${ip}`;
  const t = Math.floor(Date.now() / 1000);
  const row = await env.DB.prepare('SELECT count, window FROM rate_limits WHERE key = ?').bind(key).first();
  if (!row || t - row.window >= window) {
    await env.DB.prepare('INSERT OR REPLACE INTO rate_limits (key, count, window) VALUES (?, 1, ?)').bind(key, t).run();
    return true;
  }
  if (row.count >= max) return false;
  await env.DB.prepare('UPDATE rate_limits SET count = count + 1 WHERE key = ?').bind(key).run();
  return true;
}

/** Profile badges, worked out from what the database knows. */
export async function badges(env, user) {
  const out = [];
  if (user.id <= 100) out.push({ id: 'founder', name: 'Founder', text: 'One of the first 100 members' });
  if (user.is_admin) out.push({ id: 'dev', name: 'Developer', text: 'Builds Achievement Hub' });
  const mapper = await env.DB.prepare("SELECT 1 FROM mapper_applications WHERE user_id = ? AND status = 'accepted'").bind(user.id).first();
  if (mapper) out.push({ id: 'mapper', name: 'Mapper', text: 'Maps achievements for everyone' });
  const g = await env.DB.prepare('SELECT SUM(points) AS pts, SUM(unlocked = total AND total > 0) AS done, COUNT(*) AS games FROM player_games WHERE user_id = ?').bind(user.id).first();
  if (g && g.done) out.push({ id: 'complete', name: 'Completionist', text: `Finished ${g.done} game${g.done > 1 ? 's' : ''} 100%` });
  if (g && g.pts >= 1000) out.push({ id: 'club', name: `${Math.floor(g.pts / 1000)}K Club`, text: `${g.pts.toLocaleString()} Gamerscore` });
  if (g && g.games >= 5) out.push({ id: 'collector', name: 'Collector', text: `${g.games} games played` });
  const guides = await env.DB.prepare('SELECT COUNT(*) AS n FROM guides WHERE user_id = ?').bind(user.id).first();
  if (guides && guides.n) out.push({ id: 'guide', name: 'Guide Writer', text: `${guides.n} tip${guides.n > 1 ? 's' : ''} written` });
  return out;
}

export async function community(c) {
  const { req, env, parts, route, method, json, fail, body, currentUser } = c;
  const now = () => Math.floor(Date.now() / 1000);

  // ── Mapper applications ──
  if (route === '/mappers' && method === 'POST') {
    const b = await body(req);
    if (b.website) return json({ ok: true }); // the hidden trap field: a bot filled it in
    if (!(await rateLimit(env, req, 'mapper', 3, 86400))) return fail(429, "You've sent a few already today. We'll be in touch!");
    if (!b.consent) return fail(400, 'Please tick the box agreeing to be contacted.');
    const name = clip(b.name, 80), email = clip(b.email, 120);
    if (!name) return fail(400, 'Please tell us your name.');
    if (!/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(email)) return fail(400, "That email doesn't look right.");
    const u = await currentUser(env, req);
    await env.DB.prepare(
      'INSERT INTO mapper_applications (user_id, name, email, discord, platform, games, experience, hours, notes, created) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)',
    ).bind(u ? u.id : null, name, email, clip(b.discord, 60), clip(b.platform, 40), clip(b.games, 600), clip(b.experience, 1000), clip(b.hours, 60), clip(b.notes, 1000), now()).run();
    return json({ ok: true }, 201);
  }

  // ── Game requests ──
  if (parts[0] === 'requests') {
    const u = await currentUser(env, req);
    if (parts.length === 1 && method === 'GET') {
      const { results } = await env.DB.prepare(
        `SELECT r.id, r.title, r.store, r.notes, r.status, r.votes, r.created, us.gamertag AS by,
                EXISTS(SELECT 1 FROM request_votes v WHERE v.request_id = r.id AND v.user_id = ?) AS mine
           FROM game_requests r LEFT JOIN users us ON us.id = r.user_id
          ORDER BY r.status = 'open' DESC, r.votes DESC, r.created DESC LIMIT 200`,
      ).bind(u ? u.id : 0).all();
      return json({ requests: results });
    }
    if (!u) return fail(401, 'Log in to request or vote.');
    if (parts.length === 1 && method === 'POST') {
      const b = await body(req);
      if (b.website) return json({ ok: true });
      if (!(await rateLimit(env, req, 'request', 10, 86400))) return fail(429, "That's plenty of requests for today.");
      const title = clip(b.title, 120);
      if (title.length < 2) return fail(400, 'Which game?');
      const dupe = await env.DB.prepare('SELECT id FROM game_requests WHERE title = ? COLLATE NOCASE').bind(title).first();
      if (dupe) return fail(409, 'Someone already asked for that one; vote for it instead!');
      const r = await env.DB.prepare('INSERT INTO game_requests (title, store, notes, user_id, created, votes) VALUES (?, ?, ?, ?, ?, 1)')
        .bind(title, clip(b.store, 40), clip(b.notes, 500), u.id, now()).run();
      await env.DB.prepare('INSERT INTO request_votes (request_id, user_id) VALUES (?, ?)').bind(r.meta.last_row_id, u.id).run();
      return json({ ok: true }, 201);
    }
    if (parts[2] === 'vote' && method === 'POST') {
      const id = Number(parts[1]);
      const had = await env.DB.prepare('DELETE FROM request_votes WHERE request_id = ? AND user_id = ?').bind(id, u.id).run();
      if (!had.meta.changes) await env.DB.prepare('INSERT OR IGNORE INTO request_votes (request_id, user_id) VALUES (?, ?)').bind(id, u.id).run();
      await env.DB.prepare('UPDATE game_requests SET votes = (SELECT COUNT(*) FROM request_votes WHERE request_id = ?) WHERE id = ?').bind(id, id).run();
      return json({ ok: true, voted: !had.meta.changes });
    }
  }

  // ── Guides ──
  if (parts[0] === 'games' && parts[2] === 'guides') {
    const game = decodeURIComponent(parts[1]);
    const u = await currentUser(env, req);
    if (method === 'GET') {
      const { results } = await env.DB.prepare(
        `SELECT g.id, g.achievement, g.body, g.votes, g.created, us.gamertag AS by, us.avatar, us.color,
                EXISTS(SELECT 1 FROM guide_votes v WHERE v.guide_id = g.id AND v.user_id = ?) AS mine
           FROM guides g JOIN users us ON us.id = g.user_id WHERE g.game = ? ORDER BY g.votes DESC, g.created`,
      ).bind(u ? u.id : 0, game).all();
      return json({ guides: results });
    }
    if (method === 'POST') {
      if (!u) return fail(401, 'Log in to write a tip.');
      if (!(await rateLimit(env, req, 'guide', 30, 86400))) return fail(429, "That's a lot of tips for one day!");
      const b = await body(req);
      const text = clip(b.body, 2000);
      if (text.length < 10) return fail(400, 'Write a little more (10+ characters).');
      await env.DB.prepare('INSERT INTO guides (game, achievement, user_id, body, created) VALUES (?, ?, ?, ?, ?)')
        .bind(game, clip(b.achievement, 80), u.id, text, now()).run();
      return json({ ok: true }, 201);
    }
  }
  if (parts[0] === 'guides' && parts[1]) {
    const u = await currentUser(env, req);
    if (!u) return fail(401, 'Log in first.');
    const id = Number(parts[1]);
    if (parts[2] === 'vote' && method === 'POST') {
      const had = await env.DB.prepare('DELETE FROM guide_votes WHERE guide_id = ? AND user_id = ?').bind(id, u.id).run();
      if (!had.meta.changes) await env.DB.prepare('INSERT OR IGNORE INTO guide_votes (guide_id, user_id) VALUES (?, ?)').bind(id, u.id).run();
      await env.DB.prepare('UPDATE guides SET votes = (SELECT COUNT(*) FROM guide_votes WHERE guide_id = ?) WHERE id = ?').bind(id, id).run();
      return json({ ok: true });
    }
    if (parts.length === 2 && method === 'DELETE') {
      await env.DB.prepare('DELETE FROM guides WHERE id = ? AND (user_id = ? OR ?)').bind(id, u.id, u.is_admin ? 1 : 0).run();
      return json({ ok: true });
    }
  }

  // ── Per-game leaderboard ──
  if (parts[0] === 'games' && parts[2] === 'leaderboard' && method === 'GET') {
    const { results } = await env.DB.prepare(
      `SELECT us.gamertag, us.avatar, us.color, pg.points, pg.unlocked, pg.total FROM player_games pg JOIN users us ON us.id = pg.user_id
        WHERE pg.game = ? AND us.private = 0 ORDER BY pg.points DESC, pg.unlocked DESC LIMIT 25`,
    ).bind(decodeURIComponent(parts[1])).all();
    return json({ players: results });
  }

  // ── Friends' activity ──
  if (route === '/feed' && method === 'GET') {
    const u = await currentUser(env, req);
    if (!u) return fail(401, 'Log in to see your friends.');
    const { results } = await env.DB.prepare(
      `SELECT us.gamertag, us.avatar, us.color, p.snapshot FROM friends f
         JOIN users us ON us.id = CASE WHEN f.requester = ? THEN f.addressee ELSE f.requester END
         JOIN progress p ON p.user_id = us.id
        WHERE (f.requester = ? OR f.addressee = ?) AND f.status = 'accepted'`,
    ).bind(u.id, u.id, u.id).all();
    const items = [];
    for (const r of results) {
      for (const x of (JSON.parse(r.snapshot).recent || []).slice(0, 15)) items.push({ gamertag: r.gamertag, avatar: r.avatar, color: r.color, ...x });
    }
    items.sort((a, b) => b.time - a.time);
    return json({ items: items.slice(0, 60) });
  }

  // ── Compare with someone ──
  if (parts[0] === 'compare' && parts[1] && parts[2] && method === 'GET') {
    const u = await currentUser(env, req);
    const other = await env.DB.prepare('SELECT id, gamertag, private FROM users WHERE gamertag = ?').bind(decodeURIComponent(parts[1])).first();
    if (!other) return fail(404, 'No such gamertag.');
    const game = decodeURIComponent(parts[2]);
    if (other.private && !(u && (u.id === other.id || (await c.areFriends(env, u.id, other.id))))) return fail(403, `${other.gamertag} only shares with friends.`);
    const ids = async (id) => (await env.DB.prepare('SELECT achievement FROM player_unlocks WHERE user_id = ? AND game = ?').bind(id, game).all()).results.map((r) => r.achievement);
    return json({ them: { gamertag: other.gamertag, unlocked: await ids(other.id) }, you: u ? { gamertag: u.gamertag, unlocked: await ids(u.id) } : null });
  }

  // ── From the app: game requests and shared mappings (account optional) ──
  if (route === '/app/request' && method === 'POST') {
    if (!(await rateLimit(env, req, 'app-request', 10, 86400))) return fail(429, "That's plenty of requests for today.");
    const u = await c.appUser(env, req);
    const b = await body(req);
    const title = clip(b.game, 120);
    if (title.length < 2) return fail(400, 'Which game?');
    const notes = [clip(b.note, 400), b.exe && 'exe: ' + clip(b.exe, 200), b.anticheat && 'anti-cheat: ' + clip(b.anticheat, 100), !u && b.gamertag && 'from: ' + clip(b.gamertag, 40)]
      .filter(Boolean).join(' · ');
    const dupe = await env.DB.prepare('SELECT id FROM game_requests WHERE title = ? COLLATE NOCASE').bind(title).first();
    let id = dupe && dupe.id;
    if (!id) {
      id = (await env.DB.prepare('INSERT INTO game_requests (title, store, notes, user_id, created, votes) VALUES (?, ?, ?, ?, ?, ?)')
        .bind(title, clip(b.store, 40), notes, u ? u.id : null, now(), u ? 1 : 0).run()).meta.last_row_id;
    }
    if (u) {
      await env.DB.prepare('INSERT OR IGNORE INTO request_votes (request_id, user_id) VALUES (?, ?)').bind(id, u.id).run();
      await env.DB.prepare('UPDATE game_requests SET votes = (SELECT COUNT(*) FROM request_votes WHERE request_id = ?) WHERE id = ?').bind(id, id).run();
    }
    return json({ ok: true, id });
  }
  if (route === '/app/mapping' && method === 'POST') {
    if (!(await rateLimit(env, req, 'app-mapping', 60, 86400))) return fail(429, 'Too many for today.');
    const u = await c.appUser(env, req);
    const b = await body(req);
    const data = JSON.stringify(b.data || {});
    if (data.length > 200000 || !b.game || !b.achievement) return fail(400, 'Bad mapping.');
    await env.DB.prepare('INSERT INTO shared_mappings (user_id, gamertag, game, achievement, data, created) VALUES (?, ?, ?, ?, ?, ?)')
      .bind(u ? u.id : null, u ? u.gamertag : clip(b.gamertag, 40), clip(b.game, 80), clip(b.achievement, 80), data, now()).run();
    return json({ ok: true }, 201);
  }

  // ── Your unlocks (for picking a showcase) ──
  if (route === '/me/unlocks' && method === 'GET') {
    const u = await currentUser(env, req);
    if (!u) return fail(401, 'Not logged in.');
    const { results } = await env.DB.prepare('SELECT game, achievement FROM player_unlocks WHERE user_id = ? ORDER BY game').bind(u.id).all();
    return json({ unlocks: results });
  }

  // ── News ──
  if (route === '/news' && method === 'GET') {
    const { results } = await env.DB.prepare('SELECT id, title, body, created FROM news ORDER BY created DESC LIMIT 30').all();
    return json({ news: results });
  }

  // ── Admin ──
  if (parts[0] === 'admin') {
    const u = await currentUser(env, req);
    if (!u || !u.is_admin) return fail(403, 'Admins only.');
    if (route === '/admin/summary' && method === 'GET') {
      const mappers = (await env.DB.prepare(
        'SELECT m.*, us.gamertag FROM mapper_applications m LEFT JOIN users us ON us.id = m.user_id ORDER BY m.status = \'new\' DESC, m.created DESC',
      ).all()).results;
      const counts = await env.DB.prepare(
        'SELECT (SELECT COUNT(*) FROM users) AS users, (SELECT COUNT(*) FROM progress) AS linked, (SELECT COUNT(*) FROM game_requests WHERE status = \'open\') AS requests, (SELECT COUNT(*) FROM guides) AS guides',
      ).first();
      return json({ mappers, counts });
    }
    if (parts[1] === 'mappers' && parts[2] && method === 'PATCH') {
      const { status } = await body(req);
      if (!STATUSES.mapper.includes(status)) return fail(400, 'Bad status.');
      await env.DB.prepare('UPDATE mapper_applications SET status = ? WHERE id = ?').bind(status, Number(parts[2])).run();
      return json({ ok: true });
    }
    if (parts[1] === 'requests' && parts[2] && method === 'PATCH') {
      const { status } = await body(req);
      if (!STATUSES.request.includes(status)) return fail(400, 'Bad status.');
      await env.DB.prepare('UPDATE game_requests SET status = ? WHERE id = ?').bind(status, Number(parts[2])).run();
      return json({ ok: true });
    }
    if (route === '/admin/mappings' && method === 'GET') {
      const { results } = await env.DB.prepare('SELECT id, gamertag, game, achievement, data, created, status FROM shared_mappings ORDER BY created DESC LIMIT 500').all();
      return json({ mappings: results.map((m) => ({ ...m, data: JSON.parse(m.data) })) });
    }
    if (parts[1] === 'mappings' && parts[2] && method === 'PATCH') {
      const { status } = await body(req);
      if (!['new', 'merged', 'rejected'].includes(status)) return fail(400, 'Bad status.');
      await env.DB.prepare('UPDATE shared_mappings SET status = ? WHERE id = ?').bind(status, Number(parts[2])).run();
      return json({ ok: true });
    }
    if (parts[1] === 'news' && method === 'POST') {
      const b = await body(req);
      if (!clip(b.title, 120) || !clip(b.body, 8000)) return fail(400, 'Title and text, please.');
      await env.DB.prepare('INSERT INTO news (title, body, created) VALUES (?, ?, ?)').bind(clip(b.title, 120), clip(b.body, 8000), now()).run();
      return json({ ok: true }, 201);
    }
    if (parts[1] === 'news' && parts[2] && method === 'DELETE') {
      await env.DB.prepare('DELETE FROM news WHERE id = ?').bind(Number(parts[2])).run();
      return json({ ok: true });
    }
  }

  return null;
}
