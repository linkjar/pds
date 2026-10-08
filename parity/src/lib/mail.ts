// Mail the PDS sent, read from the stack's mail catcher (Mailpit).

import type { Scenario } from '../scenario.ts'
import { HOSTS } from '../stack/targets.ts'

export type Mail = {
  id: string
  subject: string
  from: string
  to: string[]
  text: string
  html: string
  /** Whether the message carried a plain-text and an HTML part. */
  alternative: boolean
  headers: Record<string, string[]>
}

async function api(s: Scenario, path: string): Promise<any> {
  const result = await s.http('mail', { host: HOSTS.mail, path, silent: true })
  if (result.status !== 200) throw new Error(`mail catcher ${path}: ${result.status} ${result.text}`)
  return result.json
}

/** Messages to an address, newest first. */
export async function mailsTo(s: Scenario, address: string): Promise<Mail[]> {
  const found = await api(s, `/api/v1/search?query=${encodeURIComponent(`to:"${address}"`)}`)
  const mails: Mail[] = []
  for (const summary of found.messages ?? []) {
    const message = await api(s, `/api/v1/message/${summary.ID}`)
    const headers = await api(s, `/api/v1/message/${summary.ID}/headers`)
    mails.push({
      id: summary.ID,
      subject: message.Subject,
      from: message.From?.Address,
      to: (message.To ?? []).map((to: { Address: string }) => to.Address),
      text: message.Text ?? '',
      html: message.HTML ?? '',
      alternative: Boolean(message.Text) && Boolean(message.HTML),
      headers,
    })
  }
  return mails
}

/** Waits for the `count`-th message to an address and returns the newest. */
export async function nextMail(s: Scenario, address: string, count = 1): Promise<Mail> {
  const mails = await s.eventually(`mail number ${count} to ${address}`, async () => {
    const all = await mailsTo(s, address)
    return all.length >= count && all
  })
  return mails[0]!
}

/** The `XXXXX-XXXXX` token of an account mail. */
export function mailToken(mail: Mail): string {
  const match = /\b[A-Z2-7]{5}-[A-Z2-7]{5}\b/.exec(mail.text) ?? /\b[A-Z2-7]{5}-[A-Z2-7]{5}\b/.exec(mail.html)
  if (!match) throw new Error(`No token in mail "${mail.subject}"`)
  return match[0]
}

/** What the transcript keeps of a mail: enough to hold a Candidate's templates to the same content. */
export function describeMail(mail: Mail): Record<string, unknown> {
  return {
    subject: mail.subject,
    from: mail.from,
    to: mail.to,
    alternative: mail.alternative,
    text: mail.text.replace(/\r\n/g, '\n').trim(),
  }
}
