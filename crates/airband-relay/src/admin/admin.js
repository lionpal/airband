// The admin page: sign in, see who asked for an invite, send the invites Charles chooses. Everything it shows
// comes from the relay's own answers, by their HTTP status and their typed fields; addresses are only ever put on
// the page as text.
(function () {
  const $ = (id) => document.getElementById(id);
  const gate = $("gate"), list = $("list"), rows = $("rows"), note = $("note"), listmsg = $("listmsg");
  const pickall = $("pickall"), sendchosen = $("sendchosen");

  // what came of each address's last send, when it was not a plain "sent": shown under its invite
  const notes = new Map();
  let requests = [];
  let sending = false;

  const post = (path, body) => fetch(path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body || {}) });
  const said = async (res) => (await res.json().catch(() => ({}))).message || "Something went wrong. Try again in a minute.";

  const dateTime = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
  const when = (secs) => dateTime.format(new Date(secs * 1000));

  function showGate(message) {
    list.hidden = true;
    gate.hidden = false;
    $("signinmsg").textContent = message || "";
    $("email").focus();
  }

  // the note under an invite, by the outcome the relay gave
  function noteFor(result) {
    switch (result.outcome) {
      case "not_recorded": return "Apple has it, but it wasn't saved here: " + result.why;
      case "not_set_up": return "The relay has no App Store Connect key.";
      case "failed": return "Didn't go out: " + result.why;
      case "not_on_list": return "Not on the list.";
      default: return null;
    }
  }

  function cell(tr, className, text) {
    const td = document.createElement("td");
    if (className) td.className = className;
    if (text !== undefined) td.textContent = text;
    tr.appendChild(td);
    return td;
  }

  function render() {
    const waiting = requests.filter((r) => !r.invite);
    note.textContent = requests.length === 0 ? "Nobody has asked yet." : waiting.length + " of " + requests.length + " waiting for an invite.";
    rows.replaceChildren();
    if (requests.length === 0) {
      const tr = document.createElement("tr");
      cell(tr, "empty", "Nothing yet.").colSpan = 5;
      rows.appendChild(tr);
    }
    for (const r of requests) {
      const tr = document.createElement("tr");
      const pick = cell(tr, "pick");
      if (!r.invite) {
        const box = document.createElement("input");
        box.type = "checkbox";
        box.dataset.email = r.email;
        box.setAttribute("aria-label", "Choose " + r.email);
        box.addEventListener("change", update);
        pick.appendChild(box);
      }
      cell(tr, "email", r.email);
      cell(tr, "when", when(r.joined_at));
      const invite = cell(tr);
      const state = document.createElement("div");
      if (r.invite) {
        state.className = "sent";
        state.textContent = (r.invite.how === "already_tester" ? "Already a tester, " : "Sent ") + when(r.invite.at);
      } else {
        state.className = "none";
        state.textContent = "Not sent";
      }
      invite.appendChild(state);
      if (notes.has(r.email)) {
        const why = document.createElement("div");
        why.className = "why";
        why.textContent = notes.get(r.email);
        invite.appendChild(why);
      }
      const act = cell(tr, "act");
      if (!r.invite) {
        const send = document.createElement("button");
        send.type = "button";
        send.className = "quiet";
        send.textContent = "Send invite";
        send.addEventListener("click", () => sendTo([r.email]));
        act.appendChild(send);
      }
      rows.appendChild(tr);
    }
    update();
  }

  function chosen() {
    return [...rows.querySelectorAll("input[type=checkbox]:checked")].map((b) => b.dataset.email);
  }

  function update() {
    const boxes = rows.querySelectorAll("input[type=checkbox]");
    const n = chosen().length;
    pickall.disabled = sending || boxes.length === 0;
    pickall.checked = boxes.length > 0 && n === boxes.length;
    sendchosen.disabled = sending || n === 0;
    sendchosen.textContent = n > 1 ? "Send " + n + " invites" : "Send invite";
    for (const b of rows.querySelectorAll("button")) b.disabled = sending;
  }

  async function load() {
    const res = await fetch("/admin/requests");
    if (res.status === 401) return showGate();
    if (!res.ok) {
      listmsg.textContent = await said(res);
      return;
    }
    requests = (await res.json()).requests;
    gate.hidden = true;
    list.hidden = false;
    render();
  }

  async function sendTo(emails) {
    sending = true;
    listmsg.textContent = "";
    update();
    sendchosen.textContent = emails.length > 1 ? "Sending " + emails.length + " invites…" : "Sending…";
    try {
      const res = await post("/admin/invites", { emails });
      if (res.status === 401) return showGate("Your sign-in ran out. Sign in again.");
      if (!res.ok) {
        listmsg.textContent = await said(res);
        return;
      }
      for (const result of (await res.json()).results) {
        const text = noteFor(result);
        if (text) notes.set(result.email, text);
        else notes.delete(result.email);
      }
    } catch (e) {
      listmsg.textContent = "Couldn't reach the relay. Try again in a minute.";
    } finally {
      sending = false;
    }
    await load();
  }

  $("signin").addEventListener("submit", async (e) => {
    e.preventDefault();
    const button = $("signinbtn"), msg = $("signinmsg");
    button.disabled = true;
    msg.textContent = "";
    try {
      const res = await post("/admin/sign-in", { email: $("email").value, password: $("password").value });
      if (!res.ok) {
        msg.textContent = await said(res);
        return;
      }
      $("password").value = "";
      await load();
    } catch (err) {
      msg.textContent = "Couldn't reach the relay. Try again in a minute.";
    } finally {
      button.disabled = false;
    }
  });

  $("signout").addEventListener("click", async () => {
    const res = await post("/admin/sign-out");
    if (!res.ok) {
      listmsg.textContent = await said(res);
      return;
    }
    requests = [];
    notes.clear();
    showGate();
  });

  pickall.addEventListener("change", () => {
    for (const b of rows.querySelectorAll("input[type=checkbox]")) b.checked = pickall.checked;
    update();
  });
  sendchosen.addEventListener("click", () => sendTo(chosen()));

  load().catch(() => showGate("Couldn't reach the relay. Try again in a minute."));
})();
