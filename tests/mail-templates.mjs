import assert from 'node:assert/strict';
import {ServerMailer} from '/app/packages/pds/dist/mailer/index.js';
import * as templates from '/app/packages/pds/dist/mailer/templates.js';
import nodemailer from '/app/packages/pds/node_modules/nodemailer/lib/nodemailer.js';
import fs from 'node:fs';
const config={serviceName:'LinkJar',homeUrl:'https://linkjar.io',primaryColor:'#2563eb',showBskyAppEmailConfirmationLink:false};
for(const [name,template] of Object.entries(templates)){
 const html=template({config,token:'FIXTURE-CODE',handle:'example.linkjar.social',did:'did:plc:fixture',time:'2026-10-02T09:00:00Z',change:'Apple was linked to your account.'});
 assert.ok(html.includes('<table'));assert.ok(!html.includes('height:500px'));assert.ok(html.includes('ACCOUNT SECURITY'));
 const escaped=template({config,token:'<script>bad</script>',handle:'<script>bad</script>',did:'<script>bad</script>',change:'<script>bad</script>'});assert.ok(!escaped.includes('<script>bad</script>'));
 if(process.env.MAIL_PREVIEW_DIR) fs.writeFileSync(process.env.MAIL_PREVIEW_DIR+'/'+name+'.html',html);
}
const messages=[];const transport=nodemailer.createTransport({streamTransport:true,buffer:true});const send=transport.sendMail.bind(transport);transport.sendMail=async(o)=>{const r=await send(o);messages.push(r.message.toString());return r;};
const mailer=new ServerMailer(transport,{smtpUrl:'fixture',fromAddress:'accounts@linkjar.io',disableConfirmationLink:true},{name:'LinkJar',links:[{rel:'canonical',href:'https://linkjar.io'}]});
await mailer.sendResetPassword({handle:'example.linkjar.social',token:'FIXTURE-CODE'},{to:'fixture@example.invalid'});
await mailer.sendSignInMethodChange({id:'fixture',did:'did:plc:fixture',recipient:'fixture@example.invalid',action:'linked',provider:'apple',createdAt:0});
for(const msg of messages){assert.ok(msg.includes('multipart/alternative'));assert.ok(msg.includes('text/plain'));assert.ok(msg.includes('text/html'));}
console.log('Six templates render and escape inputs; reset and security messages have HTML and plain text');
