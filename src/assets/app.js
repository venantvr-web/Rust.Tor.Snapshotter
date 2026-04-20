// Rust.Tor.Snapshotter — vanilla JS front
const $ = (s, r = document) => r.querySelector(s);
const $$ = (s, r = document) => [...r.querySelectorAll(s)];

async function api(path, opts = {}) {
  const r = await fetch(path, {
    headers: { 'Content-Type': 'application/json' },
    ...opts,
    body: opts.body !== undefined ? JSON.stringify(opts.body) : undefined,
  });
  if (!r.ok) {
    const e = await r.json().catch(() => ({}));
    throw new Error(e.error || r.statusText);
  }
  const ct = r.headers.get('content-type') || '';
  return ct.includes('application/json') ? r.json() : r.text();
}

// ---------- tabs ----------
$$('.tab').forEach(btn => btn.addEventListener('click', () => {
  const id = btn.dataset.tab;
  $$('.tab').forEach(t => t.classList.toggle('active', t === btn));
  $$('.tab-panel').forEach(p => p.classList.toggle('active', p.id === `tab-${id}`));
  if (id === 'targets') loadTargets();
  if (id === 'settings') loadSettings();
  if (id === 'snapshots') loadSnapshots();
}));

// ---------- snapshots ----------
let currentSnap = null;
let viewMode = 'rendered';

const fmtSize = n =>
  n < 1024 ? `${n} B` :
  n < 1024 * 1024 ? `${(n / 1024).toFixed(1)} KB` :
  `${(n / 1024 / 1024).toFixed(2)} MB`;

const hostOf = url => { try { return new URL(url).host; } catch { return url; } };
const fmtWhen = iso => iso.replace('T', ' ').replace('Z', '');

async function loadSnapshots() {
  const tid = $('#filter-target').value;
  const qs = tid ? `?target_id=${tid}&limit=200` : '?limit=200';
  const snaps = await api('/api/snapshots' + qs);

  const tbody = $('#snapshots tbody');
  tbody.innerHTML = '';
  if (!snaps.length) {
    $('#snap-empty').classList.remove('hidden');
  } else {
    $('#snap-empty').classList.add('hidden');
    for (const s of snaps) {
      const tr = document.createElement('tr');
      tr.dataset.id = s.id;
      tr.innerHTML = `
        <td>${fmtWhen(s.taken_at)}</td>
        <td>${hostOf(s.url)}</td>
        <td>${fmtSize(s.size_bytes)}</td>
        <td>${s.sha256 ? s.sha256.slice(0, 8) : '—'}</td>
        <td class="status-${s.status}">${s.status}</td>
      `;
      tr.addEventListener('click', () => selectSnap(s));
      tbody.appendChild(tr);
    }
  }
  refreshTargetFilter();
}

async function refreshTargetFilter() {
  const targets = await api('/api/targets');
  const sel = $('#filter-target');
  const prev = sel.value;
  sel.innerHTML = '<option value="">all targets</option>' +
    targets.map(t => `<option value="${t.id}">${hostOf(t.url)}</option>`).join('');
  sel.value = prev;
}

function selectSnap(s) {
  currentSnap = s;
  $$('#snapshots tbody tr').forEach(tr =>
    tr.classList.toggle('selected', tr.dataset.id == s.id));
  $('#viewer-title').textContent = `${s.url} — ${fmtWhen(s.taken_at)}`;
  const dl = $('#view-download');
  dl.href = `/api/snapshots/${s.id}/raw`;
  dl.hidden = false;
  renderCurrent();
}

async function renderCurrent() {
  if (!currentSnap) return;
  if (currentSnap.status !== 'ok') {
    $('#viewer-frame').classList.add('hidden');
    $('#viewer-source').classList.remove('hidden');
    $('#viewer-source').textContent = currentSnap.error || '(no content)';
    return;
  }
  if (viewMode === 'rendered') {
    $('#viewer-source').classList.add('hidden');
    $('#viewer-frame').classList.remove('hidden');
    $('#viewer-frame').src = `/api/snapshots/${currentSnap.id}/view`;
  } else {
    const src = await api(`/api/snapshots/${currentSnap.id}/raw`);
    $('#viewer-frame').classList.add('hidden');
    $('#viewer-source').classList.remove('hidden');
    $('#viewer-source').textContent = src;
  }
}

$('#view-mode-rendered').addEventListener('click', () => {
  viewMode = 'rendered';
  $('#view-mode-rendered').classList.add('active');
  $('#view-mode-source').classList.remove('active');
  renderCurrent();
});
$('#view-mode-source').addEventListener('click', () => {
  viewMode = 'source';
  $('#view-mode-source').classList.add('active');
  $('#view-mode-rendered').classList.remove('active');
  renderCurrent();
});

$('#btn-refresh').addEventListener('click', loadSnapshots);
$('#filter-target').addEventListener('change', loadSnapshots);
$('#btn-trigger').addEventListener('click', async () => {
  await api('/api/trigger', { method: 'POST' });
  $('#btn-trigger').textContent = 'triggered · refreshing soon…';
  setTimeout(() => { $('#btn-trigger').textContent = 'run now ▸'; loadSnapshots(); }, 3000);
});

// ---------- targets ----------
async function loadTargets() {
  const targets = await api('/api/targets');
  const tbody = $('#targets tbody');
  tbody.innerHTML = '';
  for (const t of targets) {
    const tr = document.createElement('tr');
    tr.innerHTML = `
      <td>${t.id}</td>
      <td>${t.url}</td>
      <td><input type="checkbox" ${t.enabled ? 'checked' : ''} data-toggle="${t.id}"></td>
      <td>${t.created_at}</td>
      <td><button class="danger" data-del="${t.id}">delete</button></td>
    `;
    tbody.appendChild(tr);
  }
  $$('[data-toggle]').forEach(cb => cb.addEventListener('change', async () => {
    await api(`/api/targets/${cb.dataset.toggle}/toggle`, {
      method: 'POST',
      body: { enabled: cb.checked },
    });
  }));
  $$('[data-del]').forEach(btn => btn.addEventListener('click', async () => {
    if (!confirm('delete this target?')) return;
    await api(`/api/targets/${btn.dataset.del}`, { method: 'DELETE' });
    loadTargets();
  }));
}

$('#target-form').addEventListener('submit', async e => {
  e.preventDefault();
  const url = $('#target-url').value.trim();
  try {
    await api('/api/targets', { method: 'POST', body: { url } });
    $('#target-url').value = '';
    loadTargets();
  } catch (err) { alert(err.message); }
});

// ---------- settings ----------
async function loadSettings() {
  const s = await api('/api/settings');
  const f = $('#settings-form');
  f.interval_secs.value = s.interval_secs;
  f.tor_socks.value = s.tor_socks;
  f.http_timeout_secs.value = s.http_timeout_secs;
  f.user_agent.value = s.user_agent;
  f.drive_folder_id.value = s.drive_folder_id;
  f.drive_enabled.checked = s.drive_enabled;
  initSaControls();
  refreshSaStatus();
}

$('#settings-form').addEventListener('submit', async e => {
  e.preventDefault();
  const f = e.target;
  const body = {
    interval_secs: +f.interval_secs.value,
    tor_socks: f.tor_socks.value,
    http_timeout_secs: +f.http_timeout_secs.value,
    user_agent: f.user_agent.value,
    drive_folder_id: f.drive_folder_id.value,
    drive_enabled: f.drive_enabled.checked,
  };
  const status = $('#settings-status');
  try {
    await api('/api/settings', { method: 'POST', body });
    status.textContent = '✓ saved';
    status.style.color = 'var(--accent)';
  } catch (err) {
    status.textContent = '✗ ' + err.message;
    status.style.color = 'var(--err)';
  }
  setTimeout(() => status.textContent = '', 3000);
});

// ---------- service account upload ----------
async function refreshSaStatus() {
  try {
    const s = await api('/api/drive/service-account');
    const dot = $('#sa-dot'), txt = $('#sa-status-text');
    if (s.present) {
      dot.className = 'sa-dot ok';
      txt.innerHTML = `loaded · <code>${s.client_email || '?'}</code>` +
        (s.project_id ? ` · project <code>${s.project_id}</code>` : '');
      $('#sa-test').disabled = false;
      $('#sa-delete').disabled = false;
    } else {
      dot.className = 'sa-dot off';
      txt.textContent = 'no service account uploaded';
      $('#sa-test').disabled = true;
      $('#sa-delete').disabled = true;
    }
  } catch (e) {
    $('#sa-status-text').textContent = '! ' + e.message;
  }
}

async function uploadSa(file) {
  const msg = $('#sa-msg');
  if (!file.name.toLowerCase().endsWith('.json')) {
    msg.textContent = '✗ expected a .json file';
    msg.style.color = 'var(--err)';
    return;
  }
  msg.textContent = 'uploading…';
  msg.style.color = 'var(--fg-dim)';
  try {
    const text = await file.text();
    const parsed = JSON.parse(text);
    if (parsed.type !== 'service_account') {
      throw new Error(`not a service_account key (type="${parsed.type}")`);
    }
    const r = await fetch('/api/drive/service-account', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: text,
    });
    if (!r.ok) {
      const err = await r.json().catch(() => ({}));
      throw new Error(err.error || r.statusText);
    }
    msg.textContent = '✓ uploaded';
    msg.style.color = 'var(--accent)';
    refreshSaStatus();
  } catch (e) {
    msg.textContent = '✗ ' + e.message;
    msg.style.color = 'var(--err)';
  }
}

function initSaControls() {
  const dz = $('#sa-dropzone');
  if (dz.dataset.wired) return;
  dz.dataset.wired = '1';

  const input = $('#sa-file');
  $('#sa-pick').addEventListener('click', e => { e.preventDefault(); input.click(); });
  input.addEventListener('change', () => { if (input.files[0]) uploadSa(input.files[0]); });

  ['dragenter', 'dragover'].forEach(ev => dz.addEventListener(ev, e => {
    e.preventDefault(); dz.classList.add('drag');
  }));
  ['dragleave', 'drop'].forEach(ev => dz.addEventListener(ev, e => {
    e.preventDefault(); dz.classList.remove('drag');
  }));
  dz.addEventListener('drop', e => {
    const f = e.dataTransfer.files[0];
    if (f) uploadSa(f);
  });
  dz.addEventListener('click', e => {
    if (e.target.id === 'sa-pick') return;
    input.click();
  });

  $('#sa-delete').addEventListener('click', async () => {
    if (!confirm('remove the stored service account?')) return;
    await api('/api/drive/service-account', { method: 'DELETE' });
    $('#sa-msg').textContent = 'removed.';
    $('#sa-msg').style.color = 'var(--fg-dim)';
    refreshSaStatus();
  });

  $('#sa-test').addEventListener('click', async () => {
    const msg = $('#sa-msg');
    msg.textContent = 'testing…';
    msg.style.color = 'var(--fg-dim)';
    try {
      const r = await api('/api/drive/test', { method: 'POST' });
      msg.textContent = `✓ upload OK (file id ${r.uploaded_file_id}, auto-deleted)`;
      msg.style.color = 'var(--accent)';
    } catch (e) {
      msg.textContent = '✗ ' + e.message;
      msg.style.color = 'var(--err)';
    }
  });
}

// ---------- boot ----------
loadSnapshots();
setInterval(() => {
  if ($('#tab-snapshots').classList.contains('active')) loadSnapshots();
}, 30000);
