export type ChatTurn = { role: 'user' | 'assistant'; content: string };
export type ChatReference = { text: string; source: string };
export type ChatSession = ChatReference & { id: string; messages: ChatTurn[]; draft: string };

export function newChat(reference?: ChatReference): ChatSession {
  return { id: `${Date.now()}-${Math.random()}`, source: reference?.source ?? '自由提问', text: reference?.text.trim() ?? '', messages: [], draft: '' };
}

export function openReference(sessions: ChatSession[], reference: ChatReference): ChatSession[] {
  const match = sessions.find(s => s.text === reference.text.trim() && s.source === reference.source);
  return match ? [match, ...sessions.filter(s => s !== match)] : [newChat(reference), ...sessions].slice(0, 12);
}

export function appendReference(session: ChatSession, reference: ChatReference): ChatSession {
  const added = reference.text.trim();
  if (!added || (session.source === reference.source && session.text === added)) return session;
  const text = [session.text, added].filter(Boolean).join('\n\n');
  if (text.length > 12000) throw new Error('合并材料不能超过 12000 字，请缩小选文范围。');
  return { ...session, text, source: session.text && session.source !== reference.source ? '多段学习材料' : reference.source };
}

export function restoreChats(raw: string): ChatSession[] {
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((s: any) => typeof s?.id === 'string' && typeof s.source === 'string' && typeof s.text === 'string' && typeof s.draft === 'string' && Array.isArray(s.messages) && s.messages.length % 2 === 0 && s.messages.every((m: any, i: number) => m?.role === (i % 2 === 0 ? 'user' : 'assistant') && typeof m.content === 'string' && !!m.content.trim()))
      .slice(0, 12).map(s => ({ ...s, messages: s.messages.slice(-60) }));
  } catch { return []; }
}

// Match the API's 24-turn/12000-character bounds, keeping full exchanges.
export function chatHistory(messages: ChatTurn[]): ChatTurn[] {
  return messages.slice(-24).map(m => ({ ...m, content: m.content.slice(0, 12000) }));
}
