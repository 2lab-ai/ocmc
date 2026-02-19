// =========================================================================
// Mission Control — app.js
// CEO override (break-glass) + control gating [mc-b1j.23]
// =========================================================================

const lanes = ["Backlog","Ready","Doing","Blocked","Done","Waiting Room"];

// Agent view scope: 'root-only' or 'all'
let agentViewScope = localStorage.getItem('agentViewScope') || 'root-only';

// Root agent id (from config; we'll determine dynamically)
let ROOT_AGENT_ID = 'main';

// ---------------------------------------------------------------------------
// Override state — kept in memory, refreshed from server
// ---------------------------------------------------------------------------
let overrideState = { active: false, session_id: null, reason: null, expires_at: null };
let overrideCountdownTimer = null;

// ---------------------------------------------------------------------------
// API helpers
// ---------------------------------------------------------------------------

async function apiGet(url){
  const r = await fetch(url);
  if(r.status===401){ window.location.href='/login'; return null; }
  if(!r.ok){ throw new Error(`GET ${url} -> ${r.status}`); }
  return await r.json();
}

async function apiPost(url, body){
  const r = await fetch(url,{method:'POST',headers:{'content-type':'application/json'},body: JSON.stringify(body||{})});
  if(r.status===401){ window.location.href='/login'; return null; }
  if(r.status===403){
    const text = await r.text();
    let parsed;
    try { parsed = JSON.parse(text); } catch(e) { parsed = { error: 'policy_denied', reason: text }; }
    if(parsed.error === 'policy_denied'){
      showPolicyDenied(parsed.reason || 'Action denied by policy.');
      return null;
    }
    throw new Error(`POST ${url} -> 403 ${text}`);
  }
  if(!r.ok){ const t = await r.text(); throw new Error(`POST ${url} -> ${r.status} ${t}`); }
  const ct = r.headers.get('content-type');
  if(ct && ct.includes('application/json')){
    return await r.json();
  }
  return true;
}

async function apiDelete(url){
  const r = await fetch(url, {method:'DELETE'});
  if(r.status===401){ window.location.href='/login'; return null; }
  if(r.status===403){
    const text = await r.text();
    let parsed;
    try { parsed = JSON.parse(text); } catch(e) { parsed = { error: 'policy_denied', reason: text }; }
    if(parsed.error === 'policy_denied'){
      showPolicyDenied(parsed.reason || 'Action denied by policy.');
      return null;
    }
    throw new Error(`DELETE ${url} -> 403 ${text}`);
  }
  if(!r.ok){ const t = await r.text(); throw new Error(`DELETE ${url} -> ${r.status} ${t}`); }
  return true;
}

// ---------------------------------------------------------------------------
// DOM helpers
// ---------------------------------------------------------------------------

function el(tag, attrs={}, children=[]){
  const e = document.createElement(tag);
  for(const [k,v] of Object.entries(attrs)){
    if(k==='class') e.className=v;
    else if(k==='text') e.textContent=v;
    else if(k==='html') e.innerHTML=v;
    else if(k==='disabled' && v) e.disabled=true;
    else e.setAttribute(k,v);
  }
  for(const c of children){
    if(typeof c === 'string') e.appendChild(document.createTextNode(c));
    else e.appendChild(c);
  }
  return e;
}

// ---------------------------------------------------------------------------
// Override banner + modal
// ---------------------------------------------------------------------------

async function refreshOverrideStatus(){
  try {
    overrideState = await apiGet('/api/override/status');
  } catch(e){
    console.warn('Failed to fetch override status:', e);
    overrideState = { active: false };
  }
  renderOverrideBanner();
}

function renderOverrideBanner(){
  const banner = document.getElementById('overrideBanner');
  if(!banner) return;

  banner.innerHTML = '';

  if(overrideState.active){
    banner.className = 'override-banner override-active';
    const remaining = overrideState.expires_at
      ? Math.max(0, Math.floor((new Date(overrideState.expires_at) - Date.now()) / 1000))
      : 0;

    banner.appendChild(el('span',{class:'override-indicator', html: '🔓 <strong>Override Active</strong>'}));
    banner.appendChild(el('span',{class:'override-reason', text: overrideState.reason || ''}));
    banner.appendChild(el('span',{class:'override-ttl', id:'overrideTtlCountdown', text: formatTtl(remaining)}));

    const disableBtn = el('button',{class:'override-disable-btn', text:'Revoke'});
    disableBtn.addEventListener('click', async ()=>{
      if(!confirm('Revoke CEO override? Actions targeting non-root agents will be blocked again.')) return;
      await apiPost('/api/override/disable');
      await refreshOverrideStatus();
      if(window._lastSnapshot) render(window._lastSnapshot);
    });
    banner.appendChild(disableBtn);

    // Start countdown
    startOverrideCountdown();
  } else {
    banner.className = 'override-banner override-inactive';
    banner.appendChild(el('span',{class:'override-indicator', text: '🔒 CEO direct control: root only'}));
    const enableBtn = el('button',{class:'override-enable-btn', text:'🔓 Override'});
    enableBtn.addEventListener('click', ()=> showOverrideModal());
    banner.appendChild(enableBtn);
  }
}

function formatTtl(seconds){
  if(seconds <= 0) return 'expired';
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  if(m > 60){
    const h = Math.floor(m / 60);
    return `${h}h ${m%60}m`;
  }
  return `${m}m ${s.toString().padStart(2,'0')}s`;
}

function startOverrideCountdown(){
  if(overrideCountdownTimer) clearInterval(overrideCountdownTimer);
  overrideCountdownTimer = setInterval(()=>{
    if(!overrideState.active || !overrideState.expires_at){
      clearInterval(overrideCountdownTimer);
      return;
    }
    const remaining = Math.max(0, Math.floor((new Date(overrideState.expires_at) - Date.now()) / 1000));
    const el = document.getElementById('overrideTtlCountdown');
    if(el) el.textContent = formatTtl(remaining);
    if(remaining <= 0){
      clearInterval(overrideCountdownTimer);
      refreshOverrideStatus().then(()=>{
        if(window._lastSnapshot) render(window._lastSnapshot);
      });
    }
  }, 1000);
}

function showOverrideModal(){
  document.getElementById('overrideModal').classList.remove('hidden');
  document.getElementById('overrideReason').value = '';
  document.getElementById('overrideTtl').value = '600';
  document.getElementById('overrideError').classList.add('hidden');
  // Reset TTL presets
  document.querySelectorAll('.ttl-btn').forEach(b => {
    b.classList.toggle('active', b.dataset.ttl === '600');
  });
  document.getElementById('overrideReason').focus();
}

function hideOverrideModal(){
  document.getElementById('overrideModal').classList.add('hidden');
}

function initOverrideModal(){
  document.getElementById('overrideModalClose').addEventListener('click', hideOverrideModal);
  document.getElementById('overrideCancel').addEventListener('click', hideOverrideModal);
  document.getElementById('overrideModal').addEventListener('click', (e)=>{
    if(e.target.classList.contains('modal-overlay')) hideOverrideModal();
  });

  // TTL presets
  document.querySelectorAll('.ttl-btn').forEach(btn => {
    btn.addEventListener('click', ()=>{
      document.querySelectorAll('.ttl-btn').forEach(b => b.classList.remove('active'));
      btn.classList.add('active');
      document.getElementById('overrideTtl').value = btn.dataset.ttl;
    });
  });

  // Submit
  document.getElementById('overrideSubmit').addEventListener('click', async ()=>{
    const reason = document.getElementById('overrideReason').value.trim();
    const ttl = parseInt(document.getElementById('overrideTtl').value, 10);
    const errorEl = document.getElementById('overrideError');

    if(!reason){
      errorEl.textContent = 'Reason is required.';
      errorEl.classList.remove('hidden');
      return;
    }
    if(isNaN(ttl) || ttl < 60){
      errorEl.textContent = 'TTL must be at least 60 seconds.';
      errorEl.classList.remove('hidden');
      return;
    }

    errorEl.classList.add('hidden');
    try {
      await apiPost('/api/override/enable', { reason, ttl_s: ttl });
      hideOverrideModal();
      await refreshOverrideStatus();
      // Re-render to update gating
      if(window._lastSnapshot) render(window._lastSnapshot);
    } catch(e){
      errorEl.textContent = e.message;
      errorEl.classList.remove('hidden');
    }
  });
}

// ---------------------------------------------------------------------------
// Policy denied modal
// ---------------------------------------------------------------------------

function showPolicyDenied(reason){
  const modal = document.getElementById('policyDeniedModal');
  document.getElementById('policyDeniedReason').textContent = reason;
  modal.classList.remove('hidden');
}

function hidePolicyDenied(){
  document.getElementById('policyDeniedModal').classList.add('hidden');
}

function initPolicyDeniedModal(){
  document.getElementById('policyDeniedClose').addEventListener('click', hidePolicyDenied);
  document.getElementById('policyDeniedDismiss').addEventListener('click', hidePolicyDenied);
  document.getElementById('policyDeniedModal').addEventListener('click', (e)=>{
    if(e.target.classList.contains('modal-overlay')) hidePolicyDenied();
  });
  document.getElementById('policyDeniedOverride').addEventListener('click', ()=>{
    hidePolicyDenied();
    showOverrideModal();
  });
}

// ---------------------------------------------------------------------------
// Control gating — determine if an action on a given target agent is allowed
// ---------------------------------------------------------------------------

/**
 * Returns true if the given agent is the root agent or null/undefined (no target).
 * When there's no active override, only root agent actions are allowed for users.
 */
function isActionAllowed(targetAgentId){
  if(!targetAgentId) return true; // no target → allowed
  if(targetAgentId === ROOT_AGENT_ID) return true; // root agent → always allowed
  if(overrideState.active) return true; // override active → allowed
  return false;
}

/**
 * Returns a gating message if the action would be denied.
 */
function gatingMessage(targetAgentId){
  if(isActionAllowed(targetAgentId)) return null;
  return `Direct control limited to root agent '${ROOT_AGENT_ID}'. Enable CEO override to control '${targetAgentId}'.`;
}

// ---------------------------------------------------------------------------
// Agent tree rendering
// ---------------------------------------------------------------------------

function buildAgentTree(agents){
  const byId = new Map();
  agents.forEach(a => byId.set(a.id, { ...a, children: [] }));
  const roots = [];
  byId.forEach(a => {
    if(a.parent_id && byId.has(a.parent_id)){
      byId.get(a.parent_id).children.push(a);
    } else {
      roots.push(a);
    }
  });
  return roots;
}

function roleBadge(role){
  const cls = {root:'role-root', pdpm:'role-pdpm', dev:'role-dev', qa:'role-qa', observer:'role-observer'};
  return el('span',{class:`role-badge ${cls[role]||'role-observer'}`, text: role});
}

function renderAgentNode(agent, depth){
  const isGated = !isActionAllowed(agent.id);
  const node = el('div',{class:'agent-node' + (isGated ? ' gated' : '')});

  const header = el('div',{class:'agent-header'},[
    roleBadge(agent.role),
    el('span',{class:'agent-name', text: agent.display_name || agent.id}),
    el('span',{class:'agent-state', text: `(${agent.state})`}),
  ]);

  if(isGated){
    header.appendChild(el('span',{class:'gated-badge', text: '🔒'}));
  }

  node.appendChild(header);

  if(agent.current_card_id){
    node.appendChild(el('div',{class:'agent-task', text: `⚡ ${agent.current_card_id}`}));
  } else {
    node.appendChild(el('div',{class:'agent-task', text: '💤 waiting-room'}));
  }

  if(isGated){
    const gateNotice = el('div',{class:'gate-notice'});
    gateNotice.appendChild(el('span',{text: '🔒 Override required for direct control'}));
    const overrideLink = el('button',{class:'gate-override-btn', text: 'Enable Override'});
    overrideLink.addEventListener('click', (e)=>{
      e.stopPropagation();
      showOverrideModal();
    });
    gateNotice.appendChild(overrideLink);
    node.appendChild(gateNotice);
  }

  if(agent.children && agent.children.length > 0){
    const childrenEl = el('div',{class:'agent-children'});
    agent.children.forEach(c => childrenEl.appendChild(renderAgentNode(c, depth+1)));
    node.appendChild(childrenEl);
  }

  return node;
}

function countDescendants(node){
  if(!node.children) return 0;
  let c = node.children.length;
  node.children.forEach(ch => c += countDescendants(ch));
  return c;
}

// ---------------------------------------------------------------------------
// View toggle
// ---------------------------------------------------------------------------

function initViewToggle(){
  const toggle = document.getElementById('agentViewToggle');
  if(!toggle) return;
  const btns = toggle.querySelectorAll('.toggle-btn');
  btns.forEach(btn => {
    if(btn.dataset.view === agentViewScope) btn.classList.add('active');
    else btn.classList.remove('active');

    btn.addEventListener('click', ()=>{
      agentViewScope = btn.dataset.view;
      localStorage.setItem('agentViewScope', agentViewScope);
      btns.forEach(b => b.classList.remove('active'));
      btn.classList.add('active');
      if(window._lastSnapshot) render(window._lastSnapshot);
    });
  });
}

// ---------------------------------------------------------------------------
// Main render
// ---------------------------------------------------------------------------

function render(snapshot){
  window._lastSnapshot = snapshot;
  document.getElementById('status').textContent = `updated ${new Date(snapshot.generated_at).toLocaleTimeString()}`;

  // Detect root agent from agents list
  const rootAgent = snapshot.agents.find(a => a.role === 'root');
  if(rootAgent) ROOT_AGENT_ID = rootAgent.id;

  // Agents — render as tree
  const agentsEl = document.getElementById('agents');
  agentsEl.innerHTML='';

  const tree = buildAgentTree(snapshot.agents);

  if(agentViewScope === 'root-only'){
    tree.forEach(root => {
      const node = el('div',{class:'agent-node'});
      const header = el('div',{class:'agent-header'},[
        roleBadge(root.role),
        el('span',{class:'agent-name', text: root.display_name || root.id}),
        el('span',{class:'agent-state', text: `(${root.state})`}),
      ]);
      node.appendChild(header);
      if(root.current_card_id){
        node.appendChild(el('div',{class:'agent-task', text: `⚡ ${root.current_card_id}`}));
      } else {
        node.appendChild(el('div',{class:'agent-task', text: '💤 waiting-room'}));
      }
      const totalDesc = countDescendants(root);
      if(totalDesc > 0){
        node.appendChild(el('div',{class:'agent-task', text: `👥 ${totalDesc} agent${totalDesc>1?'s':''} in hierarchy`}));
      }
      agentsEl.appendChild(node);
    });
  } else {
    tree.forEach(root => agentsEl.appendChild(renderAgentNode(root, 0)));
  }

  // Kanban lanes
  const kb = document.getElementById('kanban');
  kb.innerHTML='';

  const byLane = new Map();
  lanes.forEach(l=>byLane.set(l,[]));
  snapshot.tasks.forEach(t=>{
    const lane = lanes.includes(t.lane) ? t.lane : 'Ready';
    byLane.get(lane).push(t);
  });

  lanes.forEach(lane=>{
    const laneEl = el('div',{class:'lane', 'data-lane': lane});
    laneEl.appendChild(el('div',{class:'lane-title', text: lane}));

    laneEl.addEventListener('dragover', (ev)=>{ ev.preventDefault(); });
    laneEl.addEventListener('drop', async (ev)=>{
      ev.preventDefault();
      const id = ev.dataTransfer.getData('text/plain');
      if(!id) return;

      // Find the task to get its assignee for gating
      const task = snapshot.tasks.find(t => t.id === id);
      const assignee = task ? task.assignee : null;

      if(!isActionAllowed(assignee)){
        showPolicyDenied(gatingMessage(assignee));
        return;
      }

      // If override is active and target is non-root, include override context
      const body = { lane };
      if(assignee && assignee !== ROOT_AGENT_ID && overrideState.active){
        body['override'] = true;
        body.override_reason = overrideState.reason;
      }

      try {
        await apiPost(`/api/task/${encodeURIComponent(id)}/move`, body);
        await refresh();
      } catch(e){
        console.error(e);
      }
    });

    byLane.get(lane).forEach(t=> laneEl.appendChild(taskCard(t, snapshot)));
    kb.appendChild(laneEl);
  });

  // Cron
  const cron = document.getElementById('cron');
  cron.innerHTML='';
  snapshot.cron.forEach(c=>{
    const row = el('div',{class:'cron-card'},[
      el('div',{class:'row'},[
        el('div',{text: c.name}),
        el('div',{},[
          toggleBtn(c),
          runBtn(c)
        ])
      ]),
      el('div',{class:'small', text: `${c.id} — ${c.schedule} — enabled=${c.enabled} — next=${c.next_run_at_ms||'-'}`})
    ]);
    cron.appendChild(row);
  });
}

// ---------------------------------------------------------------------------
// Task card with control gating
// ---------------------------------------------------------------------------

function taskCard(t, snapshot){
  const assignee = t.assignee;
  const isGated = !isActionAllowed(assignee);

  const card = el('div',{class:'card' + (isGated ? ' card-gated' : '')});

  if(!isGated){
    card.setAttribute('draggable', 'true');
    card.addEventListener('dragstart', (ev)=>{
      card.classList.add('dragging');
      ev.dataTransfer.setData('text/plain', t.id);
    });
    card.addEventListener('dragend', ()=> card.classList.remove('dragging'));
  } else {
    // Gated cards are not draggable
    card.setAttribute('draggable', 'false');
  }

  const assignBtn = el('button',{text:'Assign'});
  if(isGated){
    assignBtn.disabled = true;
    assignBtn.className = 'btn-gated';
    assignBtn.title = gatingMessage(assignee);
  }
  assignBtn.addEventListener('click', async ()=>{
    if(isGated){
      showPolicyDenied(gatingMessage(assignee));
      return;
    }
    const a = prompt('assignee (main/opus46), empty to clear', t.assignee||'');
    if(a===null) return;

    const newAssignee = a.trim() ? a.trim() : null;
    // Check if assigning to a non-root agent requires override
    if(newAssignee && !isActionAllowed(newAssignee)){
      showPolicyDenied(gatingMessage(newAssignee));
      return;
    }

    const body = { assignee: newAssignee };
    if(newAssignee && newAssignee !== ROOT_AGENT_ID && overrideState.active){
      body['override'] = true;
      body.override_reason = overrideState.reason;
    }

    try {
      await apiPost(`/api/task/${encodeURIComponent(t.id)}/assign`, body);
      await refresh();
    } catch(e){
      console.error(e);
    }
  });

  card.appendChild(el('div',{class:'id', text: t.id}));
  card.appendChild(el('div',{class:'title', text: t.title}));

  const sub = el('div',{class:'sub'});
  const assigneeText = el('span',{text: `assignee: ${t.assignee||'-'}`});
  sub.appendChild(assigneeText);
  sub.appendChild(assignBtn);
  card.appendChild(sub);

  // Show gating indicator on gated cards
  if(isGated){
    const gateBar = el('div',{class:'card-gate-bar'});
    gateBar.appendChild(el('span',{class:'gate-icon', text: '🔒'}));
    gateBar.appendChild(el('span',{text: 'Override required'}));
    const overrideLink = el('button',{class:'gate-override-btn-small', text: 'Enable'});
    overrideLink.addEventListener('click', (e)=>{
      e.stopPropagation();
      showOverrideModal();
    });
    gateBar.appendChild(overrideLink);
    card.appendChild(gateBar);
  }

  return card;
}

// ---------------------------------------------------------------------------
// Cron buttons
// ---------------------------------------------------------------------------

function toggleBtn(c){
  const b = el('button',{text: c.enabled ? 'Disable' : 'Enable'});
  b.addEventListener('click', async ()=>{
    await apiPost(`/api/cron/${encodeURIComponent(c.id)}/toggle`, {enabled: !c.enabled});
    await refresh();
  });
  return b;
}

function runBtn(c){
  const b = el('button',{text:'Run'});
  b.addEventListener('click', async ()=>{
    if(!confirm(`Run cron now? ${c.name}`)) return;
    await apiPost(`/api/cron/${encodeURIComponent(c.id)}/run`, {});
    await refresh();
  });
  return b;
}

// ---------------------------------------------------------------------------
// Refresh + WebSocket
// ---------------------------------------------------------------------------

async function refresh(){
  try{
    await refreshOverrideStatus();
    const snap = await apiGet('/api/kanban');
    if(snap) render(snap);
  }catch(e){
    console.error(e);
    document.getElementById('status').textContent = `error: ${e.message}`;
  }
}

function connectWs(){
  try{
    const proto = location.protocol === 'https:' ? 'wss' : 'ws';
    const ws = new WebSocket(`${proto}://${location.host}/ws`);
    ws.onmessage = ()=> refresh();
    ws.onclose = ()=> setTimeout(connectWs, 2000);
  }catch(e){
    console.warn('ws connect failed', e);
  }
}

// ---------------------------------------------------------------------------
// Init
// ---------------------------------------------------------------------------

initViewToggle();
initOverrideModal();
initPolicyDeniedModal();
refresh();
connectWs();
