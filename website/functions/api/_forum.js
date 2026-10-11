// The forum, the mapping bounty board and the live unlock ticker.
//
// Forum:     GET /api/forum?tag=&q=          POST /api/forum {tag, title, body}
//            GET /api/forum/:id              POST /api/forum/:id/posts {body}
//            POST /api/forum/:id/answer {post_id}   PATCH /api/forum/:id {pinned, locked, tag} (admin)
//            DELETE /api/forum/:id           DELETE /api/forum/posts/:pid
//            POST /api/forum/posts/:pid/react {emoji}   PATCH /api/forum/posts/:pid {body}
// Bounties:  GET /api/bounties  POST /api/bounties/:game/:ach (claim)  DELETE (unclaim)
//            PATCH /api/bounties/:game/:ach {status} (claimer: submitted; admin: any)
// Counts:    GET /api/unlock-counts -> {game: {achievement: players}}
// Ticker:    GET /api/live -> the latest unlocks from public profiles

import { rateLimit } from './_community.js';
import { badText } from './_filter.js';

export const TAGS = ['announcements', 'help', 'bug', 'game-support', 'mapping', 'suggestion', 'showcase', 'general'];
export const REACTIONS = ['👍', '❤️', '😂', '🔥', '🏆', '👀'];
const clip = (s, n) => String(s ?? '').trim().slice(0, n);
const now = () => Math.floor(Date.now() / 1000);

async function isMapper(env, u) {
  if (u.is_admin) return true;
  return !!(await env.DB.prepare("SELECT 1 FROM mapper_applications WHERE user_id = ? AND status = 'accepted'").bind(u.id).first());
}

export async function forum(c) {
  const { req, env, parts, route, method, json, fail, body, currentUser } = c;

  // ── Live ticker ──
  if (route === '/live' && method === 'GET') {
    const { results } = await env.DB.prepare(
      `SELECT us.gamertag, us.avatar, us.color, p.snapshot FROM progress p JOIN users us ON us.id = p.user_id
        WHERE us.private = 0 AND p.sent > ? ORDER BY p.sent DESC LIMIT 100`,
    ).bind(now() - 30 * 86400).all();
    const items = [];
    for (const r of results) {
      for (const x of (JSON.parse(r.snapshot).recent || []).slice(0, 10)) items.push({ gamertag: r.gamertag, avatar: r.avatar, color: r.color, ...x });
    }
    items.sort((a, b) => b.time - a.time);
    return json({ items: items.slice(0, 40) }, 200, { 'cache-control': 'public, max-age=30' });
  }

  if (route === '/unlock-counts' && method === 'GET') {
    const { results } = await env.DB.prepare('SELECT game, achievement, COUNT(*) AS n FROM player_unlocks GROUP BY game, achievement').all();
    const out = {};
    for (const r of results) (out[r.game] ||= {})[r.achievement] = r.n;
    return json(out, 200, { 'cache-control': 'public, max-age=60' });
  }

  // ── Bounty board ──
  if (parts[0] === 'bounties') {
    if (parts.length === 1 && method === 'GET') {
      const u = await currentUser(env, req);
      const { results } = await env.DB.prepare(
        'SELECT b.game, b.achievement, b.claimed, b.status, us.gamertag, us.avatar, us.color FROM bounties b JOIN users us ON us.id = b.user_id',
      ).all();
      return json({ claims: results, me: u ? { gamertag: u.gamertag, mapper: await isMapper(env, u), admin: !!u.is_admin } : null });
    }
    if (parts.length === 3) {
      const u = await currentUser(env, req);
      if (!u) return fail(401, 'Log in first.');
      const game = decodeURIComponent(parts[1]), ach = decodeURIComponent(parts[2]);
      const row = await env.DB.prepare('SELECT user_id FROM bounties WHERE game = ? AND achievement = ?').bind(game, ach).first();
      if (method === 'POST') {
        if (!(await isMapper(env, u))) return fail(403, 'Claiming is for mappers. Apply on the "Become a mapper" page!');
        if (row) return fail(409, 'Someone already claimed that one.');
        const mine = await env.DB.prepare("SELECT COUNT(*) AS n FROM bounties WHERE user_id = ? AND status = 'claimed'").bind(u.id).first();
        if (mine.n >= 10) return fail(429, 'You have 10 open claims already. Finish or release some first.');
        await env.DB.prepare('INSERT INTO bounties (game, achievement, user_id, claimed) VALUES (?, ?, ?, ?)').bind(clip(game, 80), clip(ach, 80), u.id, now()).run();
        return json({ ok: true }, 201);
      }
      if (!row) return fail(404, 'Not claimed.');
      if (row.user_id !== u.id && !u.is_admin) return fail(403, "That's someone else's claim.");
      if (method === 'DELETE') {
        await env.DB.prepare('DELETE FROM bounties WHERE game = ? AND achievement = ?').bind(game, ach).run();
        return json({ ok: true });
      }
      if (method === 'PATCH') {
        const { status } = await body(req);
        const allowed = u.is_admin ? ['claimed', 'submitted', 'done'] : ['claimed', 'submitted'];
        if (!allowed.includes(status)) return fail(400, 'Bad status.');
        await env.DB.prepare('UPDATE bounties SET status = ? WHERE game = ? AND achievement = ?').bind(status, game, ach).run();
        return json({ ok: true });
      }
    }
  }

  // ── Forum ──
  if (parts[0] !== 'forum') return null;
  const u = await currentUser(env, req);

  // Reactions, edits and deletes on single posts.
  if (parts[1] === 'posts' && parts[2]) {
    if (!u) return fail(401, 'Log in first.');
    const pid = Number(parts[2]);
    const post = await env.DB.prepare('SELECT p.*, t.locked FROM forum_posts p JOIN forum_threads t ON t.id = p.thread_id WHERE p.id = ?').bind(pid).first();
    if (!post) return fail(404, 'No such post.');
    if (parts[3] === 'react' && method === 'POST') {
      const { emoji } = await body(req);
      if (!REACTIONS.includes(emoji)) return fail(400, 'Unknown reaction.');
      const had = await env.DB.prepare('DELETE FROM forum_reactions WHERE post_id = ? AND user_id = ? AND emoji = ?').bind(pid, u.id, emoji).run();
      if (!had.meta.changes) await env.DB.prepare('INSERT OR IGNORE INTO forum_reactions (post_id, user_id, emoji) VALUES (?, ?, ?)').bind(pid, u.id, emoji).run();
      return json({ ok: true, on: !had.meta.changes });
    }
    if (post.user_id !== u.id && !u.is_admin) return fail(403, "That's not your post.");
    if (parts.length === 3 && method === 'PATCH') {
      const text = clip((await body(req)).body, 8000);
      if (text.length < 2) return fail(400, 'Write something first.');
      if (badText(text)) return fail(400, badText(text));
      await env.DB.prepare('UPDATE forum_posts SET body = ?, edited = ? WHERE id = ?').bind(text, now(), pid).run();
      return json({ ok: true });
    }
    if (parts.length === 3 && method === 'DELETE') {
      const first = await env.DB.prepare('SELECT MIN(id) AS id FROM forum_posts WHERE thread_id = ?').bind(post.thread_id).first();
      if (first.id === pid) return fail(400, 'Delete the whole thread instead.');
      await env.DB.batch([
        env.DB.prepare('DELETE FROM forum_posts WHERE id = ?').bind(pid),
        env.DB.prepare('UPDATE forum_threads SET replies = MAX(replies - 1, 0), answer = CASE WHEN answer = ? THEN NULL ELSE answer END WHERE id = ?').bind(pid, post.thread_id),
      ]);
      return json({ ok: true });
    }
  }

  if (parts.length === 1 && method === 'GET') {
    const url = new URL(req.url);
    const tag = url.searchParams.get('tag') || '';
    const q = clip(url.searchParams.get('q'), 60);
    const where = [], args = [];
    if (TAGS.includes(tag)) { where.push('t.tag = ?'); args.push(tag); }
    if (q) { where.push('(t.title LIKE ? OR fp.body LIKE ?)'); args.push(`%${q}%`, `%${q}%`); }
    const { results } = await env.DB.prepare(
      `SELECT t.id, t.tag, t.title, t.created, t.last_post, t.replies, t.pinned, t.locked, t.answer IS NOT NULL AS answered,
              us.gamertag, us.avatar, us.color, us.is_admin, fp.body,
              (SELECT COUNT(*) FROM forum_reactions r WHERE r.post_id = fp.id) AS reactions,
              (SELECT emoji FROM forum_reactions r WHERE r.post_id = fp.id GROUP BY emoji ORDER BY COUNT(*) DESC LIMIT 1) AS top_emoji
         FROM forum_threads t
         JOIN forum_posts fp ON fp.id = (SELECT MIN(id) FROM forum_posts WHERE thread_id = t.id)
         LEFT JOIN users us ON us.id = t.user_id
        ${where.length ? 'WHERE ' + where.join(' AND ') : ''}
        ORDER BY t.pinned DESC, t.last_post DESC LIMIT 100`,
    ).bind(...args).all();
    for (const t of results) {
      // A picture linked in the first post becomes the thread's thumbnail.
      const img = t.body.match(/https:\/\/\S+?\.(?:png|jpe?g|gif|webp)(?:\?\S*)?(?=\s|$)/i);
      t.thumb = img ? img[0] : null;
      t.preview = t.body.replace(/\*\*|`/g, '').replace(/https:\/\/\S+/g, (u) => (u === t.thumb ? '' : u)).replace(/\s+/g, ' ').trim().slice(0, 160);
      delete t.body;
    }
    return json({ threads: results, tags: TAGS, reactions: REACTIONS });
  }

  if (parts.length === 1 && method === 'POST') {
    if (!u) return fail(401, 'Log in to start a thread.');
    if (!(await rateLimit(env, req, 'thread', 10, 86400))) return fail(429, "That's a lot of threads for one day.");
    const b = await body(req);
    if (b.website) return json({ ok: true });
    const tag = TAGS.includes(b.tag) ? b.tag : 'general';
    if (tag === 'announcements' && !u.is_admin) return fail(403, 'Only the team posts announcements.');
    const title = clip(b.title, 120), text = clip(b.body, 8000);
    if (title.length < 4) return fail(400, 'Give it a title (4+ characters).');
    if (text.length < 2) return fail(400, 'Write something first.');
    if (badText(title, text)) return fail(400, badText(title, text));
    const t = now();
    const id = (await env.DB.prepare('INSERT INTO forum_threads (tag, title, user_id, created, last_post) VALUES (?, ?, ?, ?, ?)').bind(tag, title, u.id, t, t).run()).meta.last_row_id;
    await env.DB.prepare('INSERT INTO forum_posts (thread_id, user_id, body, created) VALUES (?, ?, ?, ?)').bind(id, u.id, text, t).run();
    return json({ ok: true, id }, 201);
  }

  const id = Number(parts[1]);
  const thread = id ? await env.DB.prepare('SELECT * FROM forum_threads WHERE id = ?').bind(id).first() : null;
  if (!thread) return fail(404, 'No such thread.');

  if (parts.length === 2 && method === 'GET') {
    const { results } = await env.DB.prepare(
      `SELECT p.id, p.body, p.created, p.edited, p.user_id = ? AS mine, us.gamertag, us.avatar, us.color, us.is_admin
         FROM forum_posts p LEFT JOIN users us ON us.id = p.user_id WHERE p.thread_id = ? ORDER BY p.id LIMIT 500`,
    ).bind(u ? u.id : 0, id).all();
    const reacts = (await env.DB.prepare(
      `SELECT r.post_id, r.emoji, COUNT(*) AS n, MAX(r.user_id = ?) AS mine FROM forum_reactions r
         JOIN forum_posts p ON p.id = r.post_id WHERE p.thread_id = ? GROUP BY r.post_id, r.emoji`,
    ).bind(u ? u.id : 0, id).all()).results;
    for (const p of results) p.reactions = reacts.filter((r) => r.post_id === p.id).map(({ emoji, n, mine }) => ({ emoji, n, mine: !!mine }));
    const author = await env.DB.prepare('SELECT gamertag FROM users WHERE id = ?').bind(thread.user_id).first();
    return json({
      thread: { ...thread, gamertag: author ? author.gamertag : null, user_id: undefined },
      posts: results,
      can: { reply: !!u && (!thread.locked || !!u.is_admin), answer: !!u && (u.id === thread.user_id || !!u.is_admin), admin: !!(u && u.is_admin) },
      reactions: REACTIONS,
    });
  }
  if (!u) return fail(401, 'Log in first.');

  if (parts[2] === 'posts' && method === 'POST') {
    if (thread.locked && !u.is_admin) return fail(403, 'This thread is locked.');
    if (!(await rateLimit(env, req, 'post', 60, 3600))) return fail(429, 'Slow down a little!');
    const text = clip((await body(req)).body, 8000);
    if (text.length < 2) return fail(400, 'Write something first.');
    if (badText(text)) return fail(400, badText(text));
    const t = now();
    await env.DB.batch([
      env.DB.prepare('INSERT INTO forum_posts (thread_id, user_id, body, created) VALUES (?, ?, ?, ?)').bind(id, u.id, text, t),
      env.DB.prepare('UPDATE forum_threads SET replies = replies + 1, last_post = ? WHERE id = ?').bind(t, id),
    ]);
    return json({ ok: true }, 201);
  }
  if (parts[2] === 'answer' && method === 'POST') {
    if (u.id !== thread.user_id && !u.is_admin) return fail(403, 'Only whoever asked can pick the answer.');
    const { post_id } = await body(req);
    const pid = post_id ? Number(post_id) : null;
    if (pid && !(await env.DB.prepare('SELECT 1 FROM forum_posts WHERE id = ? AND thread_id = ?').bind(pid, id).first())) return fail(404, 'No such reply.');
    await env.DB.prepare('UPDATE forum_threads SET answer = ? WHERE id = ?').bind(pid, id).run();
    return json({ ok: true });
  }
  if (parts.length === 2 && method === 'PATCH') {
    if (!u.is_admin) return fail(403, 'Admins only.');
    const b = await body(req);
    const set = {};
    if (b.pinned !== undefined) set.pinned = b.pinned ? 1 : 0;
    if (b.locked !== undefined) set.locked = b.locked ? 1 : 0;
    if (TAGS.includes(b.tag)) set.tag = b.tag;
    const keys = Object.keys(set);
    if (keys.length) await env.DB.prepare(`UPDATE forum_threads SET ${keys.map((k) => k + ' = ?').join(', ')} WHERE id = ?`).bind(...keys.map((k) => set[k]), id).run();
    return json({ ok: true });
  }
  if (parts.length === 2 && method === 'DELETE') {
    if (!u.is_admin && (u.id !== thread.user_id || thread.replies > 0)) return fail(403, 'You can delete your thread until someone replies.');
    await env.DB.batch([
      env.DB.prepare('DELETE FROM forum_reactions WHERE post_id IN (SELECT id FROM forum_posts WHERE thread_id = ?)').bind(id),
      env.DB.prepare('DELETE FROM forum_posts WHERE thread_id = ?').bind(id),
      env.DB.prepare('DELETE FROM forum_threads WHERE id = ?').bind(id),
    ]);
    return json({ ok: true });
  }
  return null;
}
