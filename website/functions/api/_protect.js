// Protection against spam and abuse:
//
// - Turnstile (Cloudflare's bot check) on sign-up and other public forms,
//   when TURNSTILE_SITE_KEY and TURNSTILE_SECRET are set.
// - Email: required at sign-up (no throwaway inboxes). When email sending
//   is set up (RESEND_API_KEY and MAIL_FROM), a code verifies it and
//   unverified accounts can't post; "forgot password" works by email.
// - Bans, link limits for brand-new accounts, and no double posts.
//
// Settings are Cloudflare Pages environment variables; until they're set,
// the matching protection is simply off.

const now = () => Math.floor(Date.now() / 1000);

export function config(env) {
  return {
    turnstile: env.TURNSTILE_SITE_KEY || null,
    email: !!(env.RESEND_API_KEY && env.MAIL_FROM),
  };
}

/** Cloudflare Turnstile: true if the visitor passed (or it isn't set up). */
export async function human(env, req, token) {
  if (!env.TURNSTILE_SECRET) return true;
  if (!token) return false;
  const form = new FormData();
  form.append('secret', env.TURNSTILE_SECRET);
  form.append('response', String(token));
  const ip = req.headers.get('cf-connecting-ip');
  if (ip) form.append('remoteip', ip);
  try {
    const r = await fetch('https://challenges.cloudflare.com/turnstile/v0/siteverify', { method: 'POST', body: form });
    return !!(await r.json()).success;
  } catch {
    return false;
  }
}
export const NOT_HUMAN = "Please complete the \"I'm human\" check.";

// Throwaway inbox services (the common ones).
const DISPOSABLE = new Set([
  'mailinator.com', 'guerrillamail.com', 'guerrillamail.net', 'guerrillamail.org', 'sharklasers.com', 'grr.la', '10minutemail.com',
  '10minutemail.net', 'tempmail.com', 'temp-mail.org', 'temp-mail.io', 'tempmail.net', 'tempmailo.com', 'throwawaymail.com',
  'yopmail.com', 'yopmail.net', 'yopmail.fr', 'getnada.com', 'nada.email', 'trashmail.com', 'trashmail.de', 'maildrop.cc',
  'dispostable.com', 'fakeinbox.com', 'mailnesia.com', 'mintemail.com', 'mohmal.com', 'emailondeck.com', 'spamgourmet.com',
  'mailcatch.com', 'mytemp.email', 'tempinbox.com', 'burnermail.io', 'moakt.com', 'tmail.ws', 'tmpmail.org', 'tmpmail.net',
  'mail.tm', 'mailpoof.com', 'spam4.me', 'inboxkitten.com', 'emailfake.com', 'fakemail.net', 'crazymailing.com', 'discard.email',
  'mailsac.com', 'getairmail.com', 'linshiyouxiang.net', 'tempr.email', 'byom.de', 'trbvm.com', 'harakirimail.com', 'mvrht.net',
  '33mail.com', 'anonaddy.me', 'mailnull.com', 'spambox.us', 'einrot.com', 'cuvox.de', 'armyspy.com', 'dayrep.com', 'fleckens.hu',
  'gustr.com', 'jourrapide.com', 'rhyta.com', 'superrito.com', 'teleworm.us',
]);

/** Why an email address can't be used, or null. */
export function badEmail(email) {
  const e = String(email || '').trim().toLowerCase();
  if (!e) return 'An email address is required.';
  if (e.length > 120 || !/^[^@\s]+@[^@\s]+\.[a-z]{2,}$/.test(e)) return "That email doesn't look right.";
  const domain = e.split('@')[1];
  if (DISPOSABLE.has(domain) || [...DISPOSABLE].some((d) => domain.endsWith('.' + d))) return 'Please use a real email address, not a throwaway one.';
  return null;
}

/** Sends an email through Resend. Returns an error message, or null. */
export async function sendMail(env, to, subject, text) {
  if (!config(env).email) return 'Email sending is not set up yet.';
  try {
    const r = await fetch('https://api.resend.com/emails', {
      method: 'POST',
      headers: { authorization: `Bearer ${env.RESEND_API_KEY}`, 'content-type': 'application/json' },
      body: JSON.stringify({ from: env.MAIL_FROM, to: [to], subject, text }),
    });
    return r.ok ? null : `Couldn't send the email (${r.status}).`;
  } catch (e) {
    return "Couldn't send the email.";
  }
}

async function sha256(text) {
  const d = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text));
  return [...new Uint8Array(d)].map((x) => x.toString(16).padStart(2, '0')).join('');
}

/** Emails a fresh 6-digit verification code (valid 30 minutes). */
export async function sendVerification(env, user, email) {
  const code = String(crypto.getRandomValues(new Uint32Array(1))[0] % 1000000).padStart(6, '0');
  await env.DB.prepare('UPDATE users SET email_code_hash = ?, email_code_expires = ? WHERE id = ?').bind(await sha256(code), now() + 1800, user.id).run();
  return sendMail(env, email, `Your Achievement Hub code: ${code}`,
    `Hi ${user.gamertag},\n\nYour Achievement Hub verification code is ${code}\n\nIt works for 30 minutes. If you didn't sign up, you can ignore this email.\n`);
}

export async function checkCode(env, user, code) {
  if (!user.email_code_hash || user.email_code_expires < now()) return false;
  return (await sha256(String(code || '').replace(/\D/g, ''))) === user.email_code_hash;
}

export async function newResetToken(env, userId) {
  const token = [...crypto.getRandomValues(new Uint8Array(24))].map((b) => b.toString(16).padStart(2, '0')).join('');
  await env.DB.batch([
    env.DB.prepare('DELETE FROM password_resets WHERE user_id = ? OR expires < ?').bind(userId, now()),
    env.DB.prepare('INSERT INTO password_resets (token_hash, user_id, expires) VALUES (?, ?, ?)').bind(await sha256(token), userId, now() + 3600),
  ]);
  return token;
}
export async function useResetToken(env, token) {
  const row = await env.DB.prepare('SELECT user_id FROM password_resets WHERE token_hash = ? AND expires > ?').bind(await sha256(String(token || '')), now()).first();
  if (row) await env.DB.prepare('DELETE FROM password_resets WHERE user_id = ?').bind(row.user_id).run();
  return row ? row.user_id : null;
}

const LINK = /https?:\/\/|www\.|\b[a-z0-9-]+\.(com|net|org|ru|xyz|top|io|gg|ly|me|info|biz|link|click|shop)\b/i;

/**
 * Whether this user may post this text now. Returns an error message, or
 * null. `kind` names the table and column to look for an identical
 * recent post in: [table, column].
 */
export async function mayPost(env, u, texts = [], dupe = null) {
  if (!u) return 'Log in first.';
  if (u.banned) return 'This account is banned.';
  if (config(env).email && !u.email_verified && !u.is_admin) return 'Verify your email first (see the banner at the top of the page).';
  // Brand-new accounts: no links for a day (spam bots' favorite thing).
  if (!u.is_admin && now() - u.created < 86400 && texts.some((t) => LINK.test(String(t || '')))) {
    return 'New accounts can post links after their first day.';
  }
  if (dupe) {
    const [table, column, userCol = 'user_id', timeCol = 'created'] = dupe;
    const same = await env.DB.prepare(`SELECT 1 FROM ${table} WHERE ${userCol} = ? AND ${column} = ? AND ${timeCol} > ? LIMIT 1`)
      .bind(u.id, String(texts[texts.length - 1] || '').trim(), now() - 600).first();
    if (same) return 'You just posted that.';
  }
  return null;
}
