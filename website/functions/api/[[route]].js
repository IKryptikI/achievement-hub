// Achievement Hub API (Cloudflare Pages Function, D1 database "DB").
//
// Accounts:  POST /api/signup  POST /api/login  POST /api/logout
//            GET /api/me  PATCH /api/me  DELETE /api/me
// Recovery:  POST /api/recover {gamertag, recovery_code, new_password}
//            POST /api/me/recovery {password} -> a new recovery code
// Profiles:  GET /api/users/:gamertag  GET /api/leaderboard
// Games:     GET /api/games/:id/stats -> players, unlocks per achievement, yours
// Friends:   GET /api/friends  POST /api/friends {gamertag}
//            POST /api/friends/:gamertag/accept  DELETE /api/friends/:gamertag
// App link:  POST /api/link-code (website, logged in) -> {code}
//            POST /api/app/link {code} -> {token}       (the desktop app)
//            PUT /api/app/progress {snapshot}           (app/overlay, Bearer token)
//            GET /api/app/friends                       (app/overlay, Bearer token)
//            POST /api/app/reset-password {new_password} (the linked app proves it's you)
//
// Website sessions are an HttpOnly cookie; the app uses a Bearer token.
// Passwords are PBKDF2-SHA256 with a per-user salt; tokens are stored hashed.

import { community, rateLimit, badges } from './_community.js';

const SESSION_DAYS = 30;
const PBKDF2_ITERATIONS = 60000;
const now = () => Math.floor(Date.now() / 1000);

const json = (data, status = 200, headers = {}) =>
  new Response(JSON.stringify(data), { status, headers: { 'content-type': 'application/json', ...headers } });
const fail = (status, error) => json({ error }, status);

function randomToken(bytes = 32) {
  const b = crypto.getRandomValues(new Uint8Array(bytes));
  return btoa(String.fromCharCode(...b)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

async function sha256(text) {
  const d = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text));
  return [...new Uint8Array(d)].map((x) => x.toString(16).padStart(2, '0')).join('');
}

async function hashPassword(password, salt) {
  const key = await crypto.subtle.importKey('raw', new TextEncoder().encode(password), 'PBKDF2', false, ['deriveBits']);
  const bits = await crypto.subtle.deriveBits(
    { name: 'PBKDF2', hash: 'SHA-256', salt: new TextEncoder().encode(salt), iterations: PBKDF2_ITERATIONS },
    key,
    256,
  );
  return [...new Uint8Array(bits)].map((x) => x.toString(16).padStart(2, '0')).join('');
}

function cookie(req, name) {
  const m = (req.headers.get('cookie') || '').match(new RegExp('(?:^|; )' + name + '=([^;]+)'));
  return m ? m[1] : null;
}

const sessionCookie = (token, maxAge) =>
  `ah_session=${token}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=${maxAge}`;

async function currentUser(env, req) {
  const token = cookie(req, 'ah_session');
  if (!token) return null;
  return env.DB.prepare(
    'SELECT u.* FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.token_hash = ? AND s.expires > ?',
  ).bind(await sha256(token), now()).first();
}

async function appUser(env, req) {
  const m = (req.headers.get('authorization') || '').match(/^Bearer (.+)$/);
  if (!m) return null;
  const hash = await sha256(m[1]);
  const user = await env.DB.prepare('SELECT u.* FROM app_tokens t JOIN users u ON u.id = t.user_id WHERE t.token_hash = ?')
    .bind(hash).first();
  if (user) await env.DB.prepare('UPDATE app_tokens SET last_used = ? WHERE token_hash = ?').bind(now(), hash).run();
  return user;
}

async function startSession(env, userId) {
  const token = randomToken();
  await env.DB.prepare('INSERT INTO sessions (token_hash, user_id, expires) VALUES (?, ?, ?)')
    .bind(await sha256(token), userId, now() + SESSION_DAYS * 86400).run();
  return sessionCookie(token, SESSION_DAYS * 86400);
}

/** A recovery code like K7QX2-M9PLR-4WTZB-HN3CJ (shown once; stored hashed). */
function recoveryCode() {
  const A = 'ABCDEFGHJKLMNPQRSTUVWXYZ23456789';
  const b = crypto.getRandomValues(new Uint8Array(20));
  return [0, 5, 10, 15].map((i) => [...b.slice(i, i + 5)].map((x) => A[x % 32]).join('')).join('-');
}
const normCode = (c) => String(c || '').toUpperCase().replace(/[^A-Z0-9]/g, '');

async function newRecovery(env, userId) {
  const code = recoveryCode();
  await env.DB.prepare('UPDATE users SET recovery_hash = ? WHERE id = ?').bind(await sha256(normCode(code)), userId).run();
  return code;
}

async function setPassword(env, userId, password) {
  const salt = randomToken(16);
  await env.DB.batch([
    env.DB.prepare('UPDATE users SET pass_hash = ?, pass_salt = ? WHERE id = ?').bind(await hashPassword(password, salt), salt, userId),
    env.DB.prepare('DELETE FROM sessions WHERE user_id = ?').bind(userId),
  ]);
}

const GAMERTAG = /^[A-Za-z0-9 _-]{3,16}$/;
const COLOR = /^#[0-9a-fA-F]{6}$/;

/** What anyone may see about a user (progress hidden if private and not a friend). */
function publicProfile(u, progress, showProgress) {
  const p = {
    gamertag: u.gamertag,
    bio: u.bio,
    avatar: u.avatar,
    color: u.color,
    motto: u.motto,
    favorite: u.favorite,
    private: !!u.private,
    member_since: u.created,
    gamerscore: progress ? progress.gamerscore : 0,
    banner: u.banner || '',
    showcase: (() => { try { return JSON.parse(u.showcase || '[]'); } catch { return []; } })(),
  };
  if (progress && showProgress) {
    const s = JSON.parse(progress.snapshot);
    p.games = s.games || [];
    p.recent = s.recent || [];
    p.unlocked = s.unlocked || 0;
    p.playing = s.playing || null;
    p.sent = progress.sent;
  }
  return p;
}

async function areFriends(env, a, b) {
  if (!a || !b) return false;
  if (a === b) return true;
  const r = await env.DB.prepare(
    "SELECT 1 FROM friends WHERE status = 'accepted' AND ((requester = ? AND addressee = ?) OR (requester = ? AND addressee = ?))",
  ).bind(a, b, b, a).first();
  return !!r;
}

async function friendList(env, userId) {
  const { results } = await env.DB.prepare(
    `SELECT u.*, f.status, f.requester = ? AS outgoing, p.snapshot, p.gamerscore AS gs, p.sent
       FROM friends f
       JOIN users u ON u.id = CASE WHEN f.requester = ? THEN f.addressee ELSE f.requester END
       LEFT JOIN progress p ON p.user_id = u.id
      WHERE f.requester = ? OR f.addressee = ?
      ORDER BY u.gamertag`,
  ).bind(userId, userId, userId, userId).all();
  return results.map((r) => {
    const progress = r.snapshot ? { snapshot: r.snapshot, gamerscore: r.gs, sent: r.sent } : null;
    const accepted = r.status === 'accepted';
    return { ...publicProfile(r, progress, accepted), status: accepted ? 'friend' : r.outgoing ? 'sent' : 'received' };
  });
}

async function body(req) {
  try {
    return await req.json();
  } catch {
    return {};
  }
}

export async function onRequest({ request: req, env, params }) {
  const parts = params.route || [];
  const route = '/' + parts.join('/');
  const method = req.method;

  try {
    // ── Accounts ──
    if (route === '/signup' && method === 'POST') {
      const { gamertag = '', password = '', email = '', website = '' } = await body(req);
      if (website) return fail(400, 'Something went wrong.'); // the hidden trap field
      if (!(await rateLimit(env, req, 'signup', 5, 3600))) return fail(429, 'Too many sign-ups from here. Try again in an hour.');
      if (!GAMERTAG.test(gamertag)) return fail(400, 'Gamertags are 3-16 letters, numbers, spaces, - or _.');
      if (password.length < 8) return fail(400, 'Passwords need at least 8 characters.');
      if (email && !/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(email)) return fail(400, "That email doesn't look right.");
      if (await env.DB.prepare('SELECT 1 FROM users WHERE gamertag = ?').bind(gamertag.trim()).first())
        return fail(409, 'That gamertag is taken.');
      const salt = randomToken(16);
      const r = await env.DB.prepare(
        'INSERT INTO users (gamertag, email, pass_hash, pass_salt, created, avatar) VALUES (?, ?, ?, ?, ?, ?)',
      ).bind(gamertag.trim(), email || null, await hashPassword(password, salt), salt, now(), gamertag.trim()[0].toUpperCase()).run();
      const recovery_code = await newRecovery(env, r.meta.last_row_id);
      return json({ ok: true, recovery_code }, 201, { 'set-cookie': await startSession(env, r.meta.last_row_id) });
    }

    if (route === '/recover' && method === 'POST') {
      if (!(await rateLimit(env, req, 'recover', 10, 3600))) return fail(429, 'Too many tries. Wait an hour and try again.');
      const { gamertag = '', recovery_code = '', new_password = '' } = await body(req);
      const u = await env.DB.prepare('SELECT id, recovery_hash FROM users WHERE gamertag = ?').bind(gamertag.trim()).first();
      if (!u || !u.recovery_hash || (await sha256(normCode(recovery_code))) !== u.recovery_hash)
        return fail(403, "That gamertag and recovery code don't match.");
      if (new_password.length < 8) return fail(400, 'Passwords need at least 8 characters.');
      await setPassword(env, u.id, new_password);
      const fresh = await newRecovery(env, u.id);
      return json({ ok: true, recovery_code: fresh }, 200, { 'set-cookie': await startSession(env, u.id) });
    }

    if (route === '/login' && method === 'POST') {
      if (!(await rateLimit(env, req, 'login', 20, 900))) return fail(429, 'Too many tries. Wait 15 minutes and try again.');
      const { gamertag = '', password = '' } = await body(req);
      const u = await env.DB.prepare('SELECT * FROM users WHERE gamertag = ? OR email = ?').bind(gamertag.trim(), gamertag.trim()).first();
      if (!u || (await hashPassword(password, u.pass_salt)) !== u.pass_hash) return fail(401, 'Wrong gamertag or password.');
      return json({ ok: true }, 200, { 'set-cookie': await startSession(env, u.id) });
    }

    if (route === '/logout' && method === 'POST') {
      const token = cookie(req, 'ah_session');
      if (token) await env.DB.prepare('DELETE FROM sessions WHERE token_hash = ?').bind(await sha256(token)).run();
      return json({ ok: true }, 200, { 'set-cookie': sessionCookie('', 0) });
    }

    if (route === '/me/recovery' && method === 'POST') {
      const u = await currentUser(env, req);
      if (!u) return fail(401, 'Not logged in.');
      const { password = '' } = await body(req);
      if ((await hashPassword(password, u.pass_salt)) !== u.pass_hash) return fail(403, 'Your password is wrong.');
      return json({ recovery_code: await newRecovery(env, u.id) });
    }

    if (parts[0] === 'games' && parts[1] && parts[2] === 'stats' && method === 'GET') {
      const game = decodeURIComponent(parts[1]);
      const { results } = await env.DB.prepare(
        'SELECT achievement, COUNT(*) AS n FROM player_unlocks WHERE game = ? GROUP BY achievement',
      ).bind(game).all();
      const players = await env.DB.prepare('SELECT COUNT(*) AS n FROM player_games WHERE game = ?').bind(game).first();
      const viewer = await currentUser(env, req);
      let mine = [];
      if (viewer) {
        mine = (await env.DB.prepare('SELECT achievement FROM player_unlocks WHERE user_id = ? AND game = ?').bind(viewer.id, game).all())
          .results.map((r) => r.achievement);
      }
      return json({ players: players.n, unlocks: Object.fromEntries(results.map((r) => [r.achievement, r.n])), mine });
    }

    if (route === '/me') {
      const u = await currentUser(env, req);
      if (!u) return fail(401, 'Not logged in.');
      if (method === 'GET') {
        const progress = await env.DB.prepare('SELECT * FROM progress WHERE user_id = ?').bind(u.id).first();
        const apps = await env.DB.prepare('SELECT COUNT(*) AS n FROM app_tokens WHERE user_id = ?').bind(u.id).first();
        return json({ ...publicProfile(u, progress, true), email: u.email, apps_linked: apps.n, is_admin: !!u.is_admin });
      }
      if (method === 'PATCH') {
        const b = await body(req);
        const set = {};
        if (b.bio !== undefined) set.bio = String(b.bio).slice(0, 400);
        if (b.motto !== undefined) set.motto = String(b.motto).slice(0, 60);
        if (b.avatar !== undefined) set.avatar = [...String(b.avatar)].slice(0, 2).join('');
        if (b.favorite !== undefined) set.favorite = String(b.favorite).slice(0, 80);
        if (b.banner !== undefined) set.banner = String(b.banner).slice(0, 80);
        if (b.showcase !== undefined) {
          const picks = (Array.isArray(b.showcase) ? b.showcase : []).slice(0, 5).map((x) => String(x).slice(0, 160));
          set.showcase = JSON.stringify(picks);
        }
        if (b.private !== undefined) set.private = b.private ? 1 : 0;
        if (b.color !== undefined) {
          if (!COLOR.test(b.color)) return fail(400, 'Pick a color like #3fae2a.');
          set.color = b.color;
        }
        if (b.gamertag !== undefined && b.gamertag !== u.gamertag) {
          if (!GAMERTAG.test(b.gamertag)) return fail(400, 'Gamertags are 3-16 letters, numbers, spaces, - or _.');
          const taken = await env.DB.prepare('SELECT 1 FROM users WHERE gamertag = ? AND id != ?').bind(b.gamertag.trim(), u.id).first();
          if (taken) return fail(409, 'That gamertag is taken.');
          set.gamertag = b.gamertag.trim();
        }
        if (b.new_password) {
          if ((await hashPassword(b.password || '', u.pass_salt)) !== u.pass_hash) return fail(403, 'Your current password is wrong.');
          if (b.new_password.length < 8) return fail(400, 'Passwords need at least 8 characters.');
          set.pass_salt = randomToken(16);
          set.pass_hash = await hashPassword(b.new_password, set.pass_salt);
          // Other devices' sessions end (this one gets a new session below).
          await env.DB.prepare('DELETE FROM sessions WHERE user_id = ?').bind(u.id).run();
        }
        const keys = Object.keys(set);
        if (keys.length) {
          await env.DB.prepare(`UPDATE users SET ${keys.map((k) => k + ' = ?').join(', ')} WHERE id = ?`)
            .bind(...keys.map((k) => set[k]), u.id).run();
        }
        return json({ ok: true }, 200, b.new_password ? { 'set-cookie': await startSession(env, u.id) } : {});
      }
      if (method === 'DELETE') {
        const { password = '' } = await body(req);
        if ((await hashPassword(password, u.pass_salt)) !== u.pass_hash) return fail(403, 'Your password is wrong.');
        await env.DB.prepare('DELETE FROM users WHERE id = ?').bind(u.id).run();
        return json({ ok: true }, 200, { 'set-cookie': sessionCookie('', 0) });
      }
    }

    // ── Profiles ──
    if (parts[0] === 'users' && parts[1] && method === 'GET') {
      const u = await env.DB.prepare('SELECT * FROM users WHERE gamertag = ?').bind(decodeURIComponent(parts[1])).first();
      if (!u) return fail(404, 'No such gamertag.');
      const viewer = await currentUser(env, req);
      const progress = await env.DB.prepare('SELECT * FROM progress WHERE user_id = ?').bind(u.id).first();
      const friend = await areFriends(env, viewer && viewer.id, u.id);
      const p = publicProfile(u, progress, !u.private || friend);
      p.badges = await badges(env, u);
      if (viewer && viewer.id !== u.id) {
        const f = await env.DB.prepare(
          'SELECT requester, status FROM friends WHERE (requester = ? AND addressee = ?) OR (requester = ? AND addressee = ?)',
        ).bind(viewer.id, u.id, u.id, viewer.id).first();
        p.relation = !f ? 'none' : f.status === 'accepted' ? 'friend' : f.requester === viewer.id ? 'sent' : 'received';
      } else if (viewer) p.relation = 'self';
      return json(p);
    }

    if (route === '/leaderboard' && method === 'GET') {
      const { results } = await env.DB.prepare(
        'SELECT u.gamertag, u.avatar, u.color, p.gamerscore FROM progress p JOIN users u ON u.id = p.user_id WHERE u.private = 0 ORDER BY p.gamerscore DESC LIMIT 25',
      ).all();
      return json({ players: results });
    }

    // ── Friends (website) ──
    if (parts[0] === 'friends') {
      const u = await currentUser(env, req);
      if (!u) return fail(401, 'Not logged in.');
      if (parts.length === 1 && method === 'GET') return json({ friends: await friendList(env, u.id) });
      if (parts.length === 1 && method === 'POST') {
        const { gamertag = '' } = await body(req);
        const other = await env.DB.prepare('SELECT id FROM users WHERE gamertag = ?').bind(gamertag.trim()).first();
        if (!other) return fail(404, 'No such gamertag.');
        if (other.id === u.id) return fail(400, "That's you!");
        const existing = await env.DB.prepare(
          'SELECT requester, status FROM friends WHERE (requester = ? AND addressee = ?) OR (requester = ? AND addressee = ?)',
        ).bind(u.id, other.id, other.id, u.id).first();
        if (existing && existing.status === 'accepted') return fail(409, "You're already friends.");
        if (existing && existing.requester === other.id) {
          // They already asked: this accepts.
          await env.DB.prepare("UPDATE friends SET status = 'accepted' WHERE requester = ? AND addressee = ?").bind(other.id, u.id).run();
          return json({ ok: true, status: 'friend' });
        }
        if (existing) return fail(409, 'Request already sent.');
        await env.DB.prepare("INSERT INTO friends (requester, addressee, status, created) VALUES (?, ?, 'pending', ?)")
          .bind(u.id, other.id, now()).run();
        return json({ ok: true, status: 'sent' });
      }
      if (parts.length >= 2) {
        const other = await env.DB.prepare('SELECT id FROM users WHERE gamertag = ?').bind(decodeURIComponent(parts[1])).first();
        if (!other) return fail(404, 'No such gamertag.');
        if (parts[2] === 'accept' && method === 'POST') {
          const r = await env.DB.prepare("UPDATE friends SET status = 'accepted' WHERE requester = ? AND addressee = ? AND status = 'pending'")
            .bind(other.id, u.id).run();
          return r.meta.changes ? json({ ok: true }) : fail(404, 'No request from them.');
        }
        if (parts.length === 2 && method === 'DELETE') {
          // Unfriend, decline, or cancel a request.
          await env.DB.prepare('DELETE FROM friends WHERE (requester = ? AND addressee = ?) OR (requester = ? AND addressee = ?)')
            .bind(u.id, other.id, other.id, u.id).run();
          return json({ ok: true });
        }
      }
    }

    // ── Linking the desktop app ──
    if (route === '/link-code' && method === 'POST') {
      const u = await currentUser(env, req);
      if (!u) return fail(401, 'Not logged in.');
      const code = [...crypto.getRandomValues(new Uint8Array(6))].map((b) => 'ABCDEFGHJKLMNPQRSTUVWXYZ23456789'[b % 32]).join('');
      await env.DB.prepare('DELETE FROM link_codes WHERE user_id = ? OR expires < ?').bind(u.id, now()).run();
      await env.DB.prepare('INSERT INTO link_codes (code, user_id, expires) VALUES (?, ?, ?)').bind(code, u.id, now() + 600).run();
      return json({ code, expires_in: 600 });
    }

    if (route === '/app/link' && method === 'POST') {
      const { code = '' } = await body(req);
      const row = await env.DB.prepare('SELECT user_id FROM link_codes WHERE code = ? AND expires > ?').bind(code.trim().toUpperCase(), now()).first();
      if (!row) return fail(404, "That code didn't work (they last 10 minutes). Make a new one on the website.");
      await env.DB.prepare('DELETE FROM link_codes WHERE code = ?').bind(code.trim().toUpperCase()).run();
      const token = randomToken();
      await env.DB.prepare('INSERT INTO app_tokens (token_hash, user_id, created) VALUES (?, ?, ?)').bind(await sha256(token), row.user_id, now()).run();
      const u = await env.DB.prepare('SELECT gamertag FROM users WHERE id = ?').bind(row.user_id).first();
      return json({ token, gamertag: u.gamertag });
    }

    if (route === '/app/request' || route === '/app/mapping') {
      return await community({ req, env, parts, route, method, json, fail, body, currentUser, areFriends, appUser });
    }
    if (parts[0] === 'app') {
      const u = await appUser(env, req);
      if (!u) return fail(401, 'This app is not linked to an account (or was unlinked).');
      if (route === '/app/progress' && method === 'PUT') {
        const raw = await req.text();
        if (raw.length > 100000) return fail(413, 'Too large.');
        const s = JSON.parse(raw);
        const games = Array.isArray(s.games) ? s.games.slice(0, 200) : [];
        // Unlocked achievement ids, kept separately (game pages count them).
        const rows = [];
        for (const g of games) {
          for (const id of Array.isArray(g.ids) ? g.ids.slice(0, 500) : []) rows.push([String(g.id).slice(0, 80), String(id).slice(0, 80)]);
          delete g.ids;
        }
        // Per-game points for leaderboards.
        const per = games.map((g) => [String(g.id).slice(0, 80), Number(g.points) || 0, Number(g.unlocked) || 0, Number(g.total) || 0]);
        // (One statement each: the free plan allows 50 queries a request.)
        await env.DB.batch([
          env.DB.prepare('DELETE FROM player_games WHERE user_id = ?').bind(u.id),
          env.DB.prepare(
            "INSERT OR REPLACE INTO player_games (user_id, game, points, unlocked, total) SELECT ?, json_extract(value, '$[0]'), json_extract(value, '$[1]'), json_extract(value, '$[2]'), json_extract(value, '$[3]') FROM json_each(?)",
          ).bind(u.id, JSON.stringify(per)),
          env.DB.prepare('DELETE FROM player_unlocks WHERE user_id = ?').bind(u.id),
          env.DB.prepare(
            "INSERT OR IGNORE INTO player_unlocks (user_id, game, achievement) SELECT ?, json_extract(value, '$[0]'), json_extract(value, '$[1]') FROM json_each(?)",
          ).bind(u.id, JSON.stringify(rows.slice(0, 5000))),
        ]);
        const snapshot = JSON.stringify({
          games,
          recent: Array.isArray(s.recent) ? s.recent.slice(0, 50) : [],
          unlocked: Number(s.unlocked) || 0,
          playing: typeof s.playing === 'string' ? s.playing.slice(0, 120) : null,
        });
        const gamerscore = Math.max(0, Math.min(10_000_000, Number(s.gamerscore) || 0));
        await env.DB.prepare(
          'INSERT INTO progress (user_id, snapshot, gamerscore, sent) VALUES (?, ?, ?, ?) ON CONFLICT(user_id) DO UPDATE SET snapshot = excluded.snapshot, gamerscore = excluded.gamerscore, sent = excluded.sent',
        ).bind(u.id, snapshot, gamerscore, now()).run();
        return json({ ok: true, gamertag: u.gamertag });
      }
      if (route === '/app/friends' && method === 'GET') {
        return json({ gamertag: u.gamertag, friends: (await friendList(env, u.id)).filter((f) => f.status === 'friend') });
      }
      if (route === '/app/reset-password' && method === 'POST') {
        const { new_password = '' } = await body(req);
        if (new_password.length < 8) return fail(400, 'Passwords need at least 8 characters.');
        await setPassword(env, u.id, new_password);
        return json({ ok: true, gamertag: u.gamertag });
      }
      if (route === '/app/unlink' && method === 'POST') {
        await env.DB.prepare('DELETE FROM app_tokens WHERE token_hash = ?')
          .bind(await sha256((req.headers.get('authorization') || '').slice(7))).run();
        return json({ ok: true });
      }
    }

    const extra = await community({ req, env, parts, route, method, json, fail, body, currentUser, areFriends, appUser });
    if (extra) return extra;
    return fail(404, 'Not found.');
  } catch (e) {
    return fail(500, 'Server error: ' + (e && e.message ? e.message : String(e)));
  }
}
