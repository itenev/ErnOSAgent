// Ern-OS Code Extension — AI chat sidebar for code-server
// Self-contained. No build step. Connects to the running Ern-OS WebSocket.

const vscode = require('vscode');

let chatPanel;
let ws;
let currentSessionId = '';

function activate(context) {
    const provider = new ErnosChatProvider(context);
    context.subscriptions.push(
        vscode.window.registerWebviewViewProvider('ernos-chat', provider)
    );

    context.subscriptions.push(
        vscode.commands.registerCommand('ernos.sendSelection', () => {
            sendEditorSelection('');
        }),
        vscode.commands.registerCommand('ernos.explainSelection', () => {
            sendEditorSelection('Explain this code:\n');
        }),
        vscode.commands.registerCommand('ernos.fixSelection', () => {
            sendEditorSelection('Fix any issues in this code:\n');
        })
    );

    connectWs();
}

function sendEditorSelection(prefix) {
    const editor = vscode.window.activeTextEditor;
    if (!editor) return;
    const sel = editor.document.getText(editor.selection);
    if (!sel) return;
    const lang = editor.document.languageId;
    sendMessage(prefix + '```' + lang + '\n' + sel + '\n```');
}

function getWsUrl() {
    // code-server runs on 8443 by default, Ern-OS engine on 3000
    return 'ws://127.0.0.1:3000/ws';
}

function connectWs() {
    if (ws && ws.readyState <= 1) return;

    try {
        const WebSocket = require('ws');
        ws = new WebSocket(getWsUrl());
    } catch (_e) {
        // ws module may not be available in code-server's node —
        // fall back to the built-in globalThis.WebSocket if available
        try {
            ws = new WebSocket(getWsUrl());
        } catch (_e2) {
            // No WebSocket runtime — post error to webview
            postToWebview({ type: 'status', connected: false, error: 'No WebSocket runtime' });
            setTimeout(connectWs, 5000);
            return;
        }
    }

    ws.on ? wireNode(ws) : wireBrowser(ws);
}

function wireNode(sock) {
    sock.on('open', () => {
        postToWebview({ type: 'status', connected: true });
    });
    sock.on('message', (data) => {
        try { handleMsg(JSON.parse(data.toString())); } catch (_) {}
    });
    sock.on('close', () => {
        ws = null;
        postToWebview({ type: 'status', connected: false });
        setTimeout(connectWs, 5000);
    });
    sock.on('error', () => { ws = null; });
}

function wireBrowser(sock) {
    sock.onopen = () => postToWebview({ type: 'status', connected: true });
    sock.onmessage = (e) => {
        try { handleMsg(JSON.parse(e.data)); } catch (_) {}
    };
    sock.onclose = () => {
        ws = null;
        postToWebview({ type: 'status', connected: false });
        setTimeout(connectWs, 5000);
    };
    sock.onerror = () => { ws = null; };
}

function handleMsg(msg) {
    switch (msg.type) {
        case 'connected':
            postToWebview({ type: 'model_info', model: msg.model || '' });
            break;
        case 'ack':
            if (msg.session_id) currentSessionId = msg.session_id;
            break;
        case 'text_delta':
        case 'tool_executing':
        case 'tool_completed':
        case 'done':
        case 'error':
        case 'audit_running':
        case 'audit_completed':
            postToWebview(msg);
            break;
    }
}

function sendMessage(content) {
    if (!ws || (ws.readyState !== undefined && ws.readyState !== 1)) {
        vscode.window.showWarningMessage('Ern-OS: Not connected.');
        connectWs();
        return;
    }
    ws.send(JSON.stringify({ type: 'chat', content, session_id: currentSessionId || '' }));
    postToWebview({ type: 'user_message', content });
}

function postToWebview(msg) {
    if (chatPanel && chatPanel.webview) {
        chatPanel.webview.postMessage(msg);
    }
}

class ErnosChatProvider {
    constructor(context) { this._ctx = context; }

    resolveWebviewView(view) {
        chatPanel = view;
        view.webview.options = { enableScripts: true };
        view.webview.html = getWebviewHtml();
        view.webview.onDidReceiveMessage((msg) => {
            if (msg.type === 'send') sendMessage(msg.content);
            if (msg.type === 'reconnect') connectWs();
        });
        postToWebview({ type: 'status', connected: ws && ws.readyState === 1 });
    }
}

function getWebviewHtml() {
    return `<!DOCTYPE html>
<html><head>
<meta charset="UTF-8">
<style>
:root {
    --bg: var(--vscode-sideBar-background);
    --fg: var(--vscode-sideBar-foreground);
    --input-bg: var(--vscode-input-background);
    --input-fg: var(--vscode-input-foreground);
    --input-border: var(--vscode-input-border, #333);
    --accent: var(--vscode-button-background, #00ff88);
    --accent-fg: var(--vscode-button-foreground, #000);
    --muted: var(--vscode-descriptionForeground, #888);
    --border: var(--vscode-panel-border, #333);
}
* { margin:0; padding:0; box-sizing:border-box; }
body { font-family:var(--vscode-font-family,system-ui); font-size:13px; color:var(--fg); background:var(--bg); display:flex; flex-direction:column; height:100vh; }
#status { padding:6px 10px; font-size:11px; display:flex; align-items:center; gap:6px; border-bottom:1px solid var(--border); }
.dot { width:6px; height:6px; border-radius:50%; background:#666; flex-shrink:0; }
.dot.on { background:#00ff88; }
#messages { flex:1; overflow-y:auto; padding:8px; }
.m { margin-bottom:6px; padding:6px 10px; border-radius:8px; font-size:12px; line-height:1.5; white-space:pre-wrap; word-break:break-word; }
.m.u { background:var(--accent); color:var(--accent-fg); margin-left:20%; border-bottom-right-radius:2px; }
.m.a { background:var(--vscode-editor-background,#1e1e1e); border:1px solid var(--border); margin-right:10%; border-bottom-left-radius:2px; }
.m.t { background:transparent; color:var(--muted); font-size:11px; padding:2px 10px; font-style:italic; }
.m.e { background:rgba(255,68,68,0.1); color:#ff4444; border:1px solid rgba(255,68,68,0.2); }
#bar { display:flex; padding:6px; gap:4px; border-top:1px solid var(--border); }
#inp { flex:1; background:var(--input-bg); color:var(--input-fg); border:1px solid var(--input-border); border-radius:6px; padding:6px 8px; font-size:12px; font-family:inherit; resize:none; min-height:28px; max-height:100px; }
#inp:focus { outline:1px solid var(--accent); }
#btn { background:var(--accent); color:var(--accent-fg); border:none; border-radius:6px; padding:4px 10px; cursor:pointer; font-size:11px; font-weight:700; align-self:flex-end; }
</style></head><body>
<div id="status"><span class="dot" id="dot"></span><span id="stxt">Connecting...</span><span style="margin-left:auto;font-size:10px;color:var(--muted)" id="minfo"></span></div>
<div id="messages"></div>
<div id="bar"><textarea id="inp" placeholder="Ask Ern-OS..." rows="1"></textarea><button id="btn">▶</button></div>
<script>
const vscode=acquireVsCodeApi(),msgs=document.getElementById('messages'),inp=document.getElementById('inp'),dot=document.getElementById('dot'),stxt=document.getElementById('stxt'),minfo=document.getElementById('minfo');
let cur=null;
inp.addEventListener('input',()=>{inp.style.height='auto';inp.style.height=Math.min(inp.scrollHeight,100)+'px';});
inp.addEventListener('keydown',e=>{if(e.key==='Enter'&&!e.shiftKey){e.preventDefault();go();}});
document.getElementById('btn').addEventListener('click',go);
function go(){const t=inp.value.trim();if(!t)return;vscode.postMessage({type:'send',content:t});inp.value='';inp.style.height='auto';}
function add(cls,txt){const d=document.createElement('div');d.className='m '+cls;d.textContent=txt;msgs.appendChild(d);msgs.scrollTop=msgs.scrollHeight;return d;}
window.addEventListener('message',e=>{const d=e.data;
if(d.type==='status'){dot.className='dot'+(d.connected?' on':'');stxt.textContent=d.connected?'Connected':'Disconnected';}
else if(d.type==='model_info'){minfo.textContent=d.model||'';}
else if(d.type==='user_message'){add('u',d.content);cur=null;}
else if(d.type==='text_delta'){if(!cur)cur=add('a','');cur.textContent+=d.content;msgs.scrollTop=msgs.scrollHeight;}
else if(d.type==='tool_executing'){add('t','⚙️ '+d.name+'...');}
else if(d.type==='tool_completed'){add('t',(d.success?'✅':'❌')+' '+d.name);}
else if(d.type==='done'){cur=null;}
else if(d.type==='error'){add('e','❌ '+d.message);cur=null;}
else if(d.type==='audit_running'){add('t','🔍 Observer...');}
else if(d.type==='audit_completed'&&!d.approved){add('t','⚠️ '+(d.reason||'Retry'));}
});
</script></body></html>`;
}

module.exports = { activate, deactivate: () => { if (ws) { ws.close(); ws = null; } } };
