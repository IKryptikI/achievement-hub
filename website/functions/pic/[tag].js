// A player's uploaded gamer picture: /pic/<gamertag> (?v=<version> for caching).

export async function onRequestGet({ env, params }) {
  const row = await env.DB.prepare('SELECT g.img, g.type, g.updated FROM gamerpics g JOIN users u ON u.id = g.user_id WHERE u.gamertag = ?')
    .bind(decodeURIComponent(params.tag || '')).first();
  if (!row) return new Response('No picture', { status: 404 });
  return new Response(new Uint8Array(row.img), {
    headers: { 'content-type': row.type, 'cache-control': 'public, max-age=300', etag: `"${row.updated}"` },
  });
}
