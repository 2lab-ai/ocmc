const lanes = ["Backlog","Ready","Doing","Blocked","Done","Waiting Room"];

// Agent view scope: 'root-only' or 'all'
let agentViewScope = localStorage.getItem('agentViewScope') || 'root-only';

async function apiGet(url){
  const r = await fetch(url);
  if(r.status===401){ window.location.href='/login'; return null; }
  if(!r.ok){ throw new Error(`GET ${url} -> ${r.status}`); }
  return await r.json();
}

async function apiPost(url, body){
  const r = await fetch(url,{method:'POST',headers:{'content-type':'application/json'},body: JSON.stringify(body||{})});
  if(r.status===401){ window.location.href='/login'; return null; }
  if(!r.ok){ const t = await r.text(); throw new Error(`POST ${url} -> ${r.status} ${t}`); }
  return true;
}

function el(tag, attrs={}, children=[]){
  const e = document.createElement(tag);
  for(const [k,v] of Object.entries(attrs)){
    if(k==='class') e.className=v;
    else if(k==='text') e.textContent=v;
    else e.setAttribute(k,v);
  }
  for(const c of children) e.appendChild(c);
  return e;
}

function buildAgentTree(agents){
  // Build a tree from flat list using parent_id.
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
  const node = el('div',{class:'agent-node'});

  const header = el('div',{class:'agent-header'},[
    roleBadge(agent.role),
    el('span',{class:'agent-name', text: agent.display_name || agent.id}),
    el('span',{class:'agent-state', text: `(${agent.state})`}),
  ]);
  node.appendChild(header);

  if(agent.current_card_id){
    node.appendChild(el('div',{class:'agent-task', text: `⚡ ${agent.current_card_id}`}));
  } else {
    node.appendChild(el('div',{class:'agent-task', text: '💤 waiting-room'}));
  }

  if(agent.children && agent.children.length > 0){
    const childrenEl = el('div',{class:'agent-children'});
    agent.children.forEach(c => childrenEl.appendChild(renderAgentNode(c, depth+1)));
    node.appendChild(childrenEl);
  }

  return node;
}

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

function render(snapshot){
  window._lastSnapshot = snapshot;
  document.getElementById('status').textContent = `updated ${new Date(snapshot.generated_at).toLocaleTimeString()}`;

  // Agents — render as tree
  const agentsEl = document.getElementById('agents');
  agentsEl.innerHTML='';

  const tree = buildAgentTree(snapshot.agents);

  if(agentViewScope === 'root-only'){
    // Show only root agents (no children expanded inline, but show direct children count)
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
      // Show count of descendants
      const totalDesc = countDescendants(root);
      if(totalDesc > 0){
        node.appendChild(el('div',{class:'agent-task', text: `👥 ${totalDesc} agent${totalDesc>1?'s':''} in hierarchy`}));
      }
      agentsEl.appendChild(node);
    });
  } else {
    // Show full tree
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
      await apiPost(`/api/task/${encodeURIComponent(id)}/move`, {lane});
      await refresh();
    });

    byLane.get(lane).forEach(t=> laneEl.appendChild(taskCard(t)));
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

function taskCard(t){
  const card = el('div',{class:'card', draggable:'true'});
  card.addEventListener('dragstart', (ev)=>{
    card.classList.add('dragging');
    ev.dataTransfer.setData('text/plain', t.id);
  });
  card.addEventListener('dragend', ()=> card.classList.remove('dragging'));

  const assign = el('button',{text:'Assign'});
  assign.addEventListener('click', async ()=>{
    const a = prompt('assignee (main/opus46), empty to clear', t.assignee||'');
    if(a===null) return;
    await apiPost(`/api/task/${encodeURIComponent(t.id)}/assign`, {assignee: a.trim() ? a.trim() : null});
    await refresh();
  });

  card.appendChild(el('div',{class:'id', text: t.id}));
  card.appendChild(el('div',{class:'title', text: t.title}));
  card.appendChild(el('div',{class:'sub'},[
    el('span',{text: `assignee: ${t.assignee||'-'}`}),
    assign
  ]));
  return card;
}

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

async function refresh(){
  try{
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

function countDescendants(node){
  if(!node.children) return 0;
  let c = node.children.length;
  node.children.forEach(ch => c += countDescendants(ch));
  return c;
}

initViewToggle();
refresh();
connectWs();
