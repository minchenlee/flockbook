// Flockbook service worker: shows the hub's Web Push messages and opens the agent when one is tapped. No fetch
// handler and no cache: the page always comes from the hub. A classic script (no imports), served as /sw.js.
//
// A message is JSON from the Mac (`PushNotice` in HerdrKit): {title, body?, tag?, pane?}. It holds no conversation
// text by default. Every push shows a notification (iOS revokes the subscription of a worker that shows none).

// The worker types this file uses (the shared deno config has the DOM lib, not the webworker lib).
                                                                                                
                                                                                                  
                                                                                                            
                   
                                          
                     
            
                                                                                             
                                              
                           
    
                               
                                                                    
                                                                                         
                                                                                                                 
 
const sw = self                      ;

/** The notification for one push message. Text is cut to what a lock screen shows; a bad message still shows one. */
function swNotice(data         )                                                  {
  const d = (data && typeof data === "object" ? data : {})                           ;
  const text = (v         , max        )         => (typeof v === "string" ? v.trim().slice(0, max) : "");
  const pane = text(d.pane, 200);
  const tag = text(d.tag, 200);
  return {
    title: text(d.title, 120) || "Flockbook",
    options: { body: text(d.body, 200), icon: "icon-192.png", tag: tag || undefined, data: { pane: pane || null } },
  };
}

/** The page for a tapped notification: the agent's conversation, else the list. */
function swTarget(pane         )         {
  return typeof pane === "string" && pane ? `./#pane=${encodeURIComponent(pane)}` : "./";
}

sw.addEventListener("install", (e) => e.waitUntil(sw.skipWaiting()));
sw.addEventListener("activate", (e) => e.waitUntil(sw.clients.claim()));

sw.addEventListener("push", (e) => {
  let data          = null;
  try { data = e.data?.json() ?? null; } catch { /* not JSON: the default notice */ }
  const n = swNotice(data);
  e.waitUntil(sw.registration.showNotification(n.title, n.options));
});

sw.addEventListener("notificationclick", (e) => {
  e.notification.close();
  const url = new URL(swTarget((e.notification.data                             )?.pane), sw.registration.scope).href;
  e.waitUntil((async () => {
    const open = await sw.clients.matchAll({ type: "window", includeUncontrolled: true });
    const page = open.find((c) => c.url.startsWith(sw.registration.scope));
    if (page) { await page.focus(); await page.navigate(url).catch(() => {}); return; }
    await sw.clients.openWindow(url);
  })());
});
