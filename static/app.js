const lanes = ["Backlog","Ready","Doing","Blocked","Done","Waiting Room"];

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

function render(snapshot){
  document.getElementById('status').textContent = `updated ${new Date(snapshot.generated_at).toLocaleTimeString()}`;

  // Agents
  const agents = document.getElementById('agents');
  agents.innerHTML='';
  snapshot.agents.forEach(a=>{
    agents.appendChild(el('div',{class:'agent'},[
      el('div',{text: `${a.display_name} (${a.state})`}),
      el('div',{class:'meta', text: a.current_card_id || 'waiting-room'})
    ]));
  });

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

refresh();
connectWs();
