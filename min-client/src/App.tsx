import { FormEvent, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  KeyRound,
  Lock,
  MessageSquareText,
  PlugZap,
  Send,
  ShieldCheck,
  UserRound,
  Users,
} from "lucide-react";

type StoredMessage = {
  id: string;
  peer_id: string;
  direction: "inbound" | "outbound";
  body: string;
  created_at: string;
};

type Identity = {
  id: string;
  display_name: string;
  public_key: string;
};

export default function App() {
  const [identity, setIdentity] = useState<Identity | null>(null);
  const [displayName, setDisplayName] = useState("min user");
  const [serverUrl, setServerUrl] = useState("ws://127.0.0.1:3027/ws");
  const [recipientId, setRecipientId] = useState("");
  const [message, setMessage] = useState("");
  const [messages, setMessages] = useState<StoredMessage[]>([]);
  const [status, setStatus] = useState("offline");
  const [onlineUsers, setOnlineUsers] = useState<Identity[]>([]);

  const timelineRef = useRef<HTMLDivElement>(null);

  const activeMessages = useMemo(
    () => messages.filter((item) => !recipientId || item.peer_id === recipientId),
    [messages, recipientId],
  );

  useEffect(() => {
    void refreshMessages();
  }, []);

  useEffect(() => {
    const unlistenPromise = listen("new-message", () => {
      void refreshMessages();
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    if (timelineRef.current) {
      timelineRef.current.scrollTop = timelineRef.current.scrollHeight;
    }
  }, [activeMessages]);

  // Poll online users every 3 seconds when connected
  useEffect(() => {
    if (status !== "connected") return;

    const poll = async () => {
      try {
        const users = await invoke<Identity[]>("get_online_users", {
          server_url: serverUrl,
        });
        // Filter out ourselves from the list
        setOnlineUsers(
          identity ? users.filter((u) => u.id !== identity.id) : users,
        );
      } catch {
        // silently ignore polling errors
      }
    };

    void poll();
    const interval = setInterval(poll, 3000);
    return () => clearInterval(interval);
  }, [status, serverUrl, identity]);

  async function refreshMessages() {
    const rows = await invoke<StoredMessage[]>("list_messages");
    setMessages(rows);
  }

  async function createIdentity() {
    const next = await invoke<Identity>("create_identity", { display_name: displayName });
    setIdentity(next);
    setStatus("identity ready");
  }

  async function connect() {
    if (!identity) return;
    await invoke("connect_server", {
      server_url: serverUrl,
      user_id: identity.id,
      display_name: identity.display_name,
      public_key: identity.public_key,
    });
    setStatus("connected");
  }

  async function sendMessage(event: FormEvent) {
    event.preventDefault();
    if (!recipientId.trim() || !message.trim()) return;
    await invoke("send_message", {
      recipient_id: recipientId.trim(),
      body: message.trim(),
    });
    setMessage("");
    await refreshMessages();
  }

  function selectUser(userId: string) {
    setRecipientId(userId);
  }

  return (
    <main className="shell">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark">m</div>
          <div>
            <h1>min</h1>
            <span>{status}</span>
          </div>
        </div>

        <section className="panel identity-panel">
          <div className="panel-title">
            <UserRound size={17} />
            <span>Identity</span>
          </div>
          <input
            value={displayName}
            onChange={(event) => setDisplayName(event.target.value)}
            placeholder="Display name"
          />
          <button onClick={createIdentity}>
            <KeyRound size={16} />
            Generate
          </button>
          {identity && (
            <div className="fingerprint">
              <span>{identity.id}</span>
              <code>{identity.public_key.slice(0, 28)}...</code>
            </div>
          )}
        </section>

        <section className="panel">
          <div className="panel-title">
            <PlugZap size={17} />
            <span>Server</span>
          </div>
          <input
            value={serverUrl}
            onChange={(event) => setServerUrl(event.target.value)}
            placeholder="ws://host:port/ws"
          />
          <button disabled={!identity} onClick={connect}>
            <ShieldCheck size={16} />
            Connect
          </button>
        </section>

        {status === "connected" && (
          <section className="panel online-panel">
            <div className="panel-title">
              <Users size={17} />
              <span>Online</span>
              <span className="online-count">{onlineUsers.length}</span>
            </div>
            {onlineUsers.length === 0 ? (
              <p className="no-users">No other users online</p>
            ) : (
              <ul className="user-list">
                {onlineUsers.map((user) => (
                  <li
                    key={user.id}
                    className={`user-item${recipientId === user.id ? " active" : ""}`}
                    onClick={() => selectUser(user.id)}
                  >
                    <span className="online-dot" />
                    <div className="user-info">
                      <span className="user-name">{user.display_name}</span>
                      <span className="user-id">{user.id}</span>
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </section>
        )}
      </aside>

      <section className="conversation">
        <header className="topbar">
          <div>
            <p className="eyebrow">
              <Lock size={14} />
              Noise XX + AES-GCM
            </p>
            <h2>Secure channel</h2>
          </div>
          <div className="recipient">
            <MessageSquareText size={16} />
            <input
              value={recipientId}
              onChange={(event) => setRecipientId(event.target.value)}
              placeholder="recipient id"
            />
          </div>
        </header>

        <div className="timeline" ref={timelineRef}>
          {activeMessages.map((item) => (
            <article className={`bubble ${item.direction}`} key={item.id}>
              <p>{item.body}</p>
              <span>{new Date(item.created_at).toLocaleTimeString()}</span>
            </article>
          ))}
          {activeMessages.length === 0 && (
            <div className="empty">
              <Lock size={32} />
              <p>No messages in this thread</p>
            </div>
          )}
        </div>

        <form className="composer" onSubmit={sendMessage}>
          <input
            value={message}
            onChange={(event) => setMessage(event.target.value)}
            placeholder="Encrypted message"
          />
          <button type="submit">
            <Send size={17} />
            Send
          </button>
        </form>
      </section>
    </main>
  );
}
