const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const ts = require('typescript');
const exportsObject = {};
vm.runInNewContext(ts.transpileModule(fs.readFileSync(path.join(__dirname,'../shared/data/subtitles.ts'),'utf8'),{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020}}).outputText,{exports:exportsObject});
const {mergeSubtitleLanguage:merge,serializeSubtitleCues:serialize,parseSubtitleCues:parse}=exportsObject;
const cue=(start,end,text)=>({id:String(start),start,end,text});
test('Chinese import preserves English and replaces previous Chinese',()=>{
 const result=merge([cue(0,3,'Hello\n旧翻译')],[cue(0,3,'你好')],'zh');
 assert.equal(result[0].text,'Hello\n你好'); assert.equal(result[0].start,0);assert.equal(result[0].end,3);
});
test('English import preserves existing Chinese',()=>{
 assert.equal(merge([cue(0,2,'你好')],[cue(0,2,'Hello')],'en')[0].text,'Hello\n你好');
});
test('different cue boundaries align by time rather than line number and retain unmatched text',()=>{
 const result=merge([cue(0,4,'Hello'),cue(5,7,'Bye')],[cue(1,2,'你好'),cue(2,4,'您好')],'zh');
 assert.equal(result.map(c=>`${c.start}-${c.end}:${c.text}`).join('|'),'0-1:Hello|1-2:Hello\n你好|2-4:Hello\n您好|5-7:Bye');
});
test('wrong language and invalid timestamps reject without modifying existing cues',()=>{
 const base=[cue(0,2,'Hello')];
 assert.throws(()=>merge(base,[cue(0,2,'World')],'zh'),/没有可用/);
 assert.throws(()=>merge(base,[cue(2,1,'你好')],'zh'),/没有可用/);
 assert.equal(base[0].text,'Hello');
});
test('merged subtitles round trip to SRT preserving millisecond timestamps',()=>{
 const merged=merge([cue(1.125,3.999,'Hello')],[cue(1.125,3.999,'你好')],'zh');
 const parsed=parse(serialize(merged));assert.equal(parsed[0].text,'Hello\n你好');assert.equal(parsed[0].start,1.125);assert.equal(parsed[0].end,3.999);
});
