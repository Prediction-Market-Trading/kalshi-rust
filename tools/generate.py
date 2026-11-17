#!/usr/bin/env python3
"""Generate independent serde bindings from pinned Kalshi JSON specifications.

Uses only Python's standard library. Run from any directory, then cargo fmt.
This is deliberately a small generator for the schema constructs used by the
pinned snapshots. Unknown constructs fail rather than producing empty stubs.
"""
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

def pascal(s):
    return ''.join(x[:1].upper() + x[1:] for x in re.split(r'[^a-zA-Z0-9]', s) if x)

def snake(s):
    s = re.sub(r'([A-Z]+)([A-Z][a-z])', r'\1_\2', s)
    s = re.sub(r'([a-z0-9])([A-Z])', r'\1_\2', s)
    s = re.sub(r'[^a-zA-Z0-9_]', '_', s).lower()
    return 'r#' + s if s in {'type', 'self', 'ref', 'match', 'default', 'enum', 'in', 'override'} else s

def doc(s, indent=''):
    # Plain text avoids broken Rustdoc links copied from website-relative Markdown.
    s = re.sub(r'\[([^\]]+)\]\([^)]+\)', r'\1', s or '')
    s = s.replace('[', r'\[').replace(']', r'\]')
    s = re.sub(r'(?<![<`])https?://[^\s<>`]*[a-zA-Z0-9/]', r'<\g<0>>', s)
    return '\n'.join(indent + '/// ' + line for line in s.splitlines()) + '\n'

class Models:
    def __init__(self, spec, ws=False, margin=False):
        self.schemas = spec['components']['schemas']
        self.ws = ws
        self.margin = margin
        self.defs = {}
        self.busy = set()

    def refname(self, ref):
        return pascal(ref.rsplit('/', 1)[-1])

    def nullable(self, s, seen=None):
        if s.get('nullable', False) or isinstance(s.get('type'), list) and 'null' in s['type']:
            return True
        seen = set() if seen is None else seen
        if '$ref' in s and s['$ref'] not in seen:
            return self.nullable(self.schemas[s['$ref'].rsplit('/', 1)[-1]], seen | {s['$ref']})
        return any(self.nullable(v, seen) for v in s.get('allOf', []))

    def typ(self, s, name, field=''):
        if '$ref' in s:
            return self.refname(s['$ref'])
        if 'allOf' in s:
            assert len(s['allOf']) == 1, s
            return self.typ(s['allOf'][0], name, field)
        for union in ['oneOf', 'anyOf']:
            if union in s:
                raise ValueError(f'Unsupported {union} in {name}; extend the generator explicitly')
        t = s.get('type')
        if isinstance(t, list):
            t = next(x for x in t if x != 'null')
        if 'enum' in s and t == 'string':
            self.emit(name, s)
            return name
        if t == 'object' or 'properties' in s:
            if s.get('properties'):
                self.emit(name, s)
                return name
            ap = s.get('additionalProperties', True)
            v = self.typ(ap, name + 'Value') if isinstance(ap, dict) else 'serde_json::Value'
            return f'std::collections::BTreeMap<String, {v}>'
        if t == 'array':
            # The two slots in orderbook levels are different units.
            if s.get('minItems') == s.get('maxItems') == 2 and ('price' in field or 'dollars' in field or name == 'PriceLevelDollarsCountFp'):
                return '(FixedPointDollars, FixedPointCount)'
            return 'Vec<' + self.typ(s['items'], name + 'Item', field) + '>'
        if self.ws and t == 'string':
            if self.margin and ('fixed_point_count' in s.get('x-rust-serialize-with', '') or s.get('x-rust-type') in {'common::types::Amount', 'common::types::Position'}):
                return 'FixedPointCount'
            if field.endswith('_fp') or field in {'delta_fp', 'post_position_fp'}:
                return 'FixedPointCount'
            if field.endswith('_dollars') or field == 'fee_cost' or s.get('x-rust-type') in {'common::types::Price', 'common::types::Value'}:
                return 'FixedPointDollars'
        if t == 'string': return 'String'
        if t == 'integer': return 'i64'
        if t == 'number': return 'serde_json::Number'
        if t == 'boolean': return 'bool'
        if not s or set(s) <= {'description'}: return 'serde_json::Value'
        raise ValueError((name, s))

    def emit(self, name, s):
        if name in self.defs or name in self.busy: return
        self.busy.add(name)
        d = doc(s.get('description') or f'{name} wire representation from the official specification.')
        if (s.get('type') == 'string' or isinstance(s.get('type'), list) and 'string' in s['type']) and 'enum' in s:
            values = [v for v in s['enum'] if v is not None]
            variants = []
            for v in values:
                variant = pascal(v) or 'Empty'
                if variant == 'Self': variant = 'SelfOnly'
                if variant == 'Unknown': variant = 'UnknownStatus'
                if variant[0].isdigit(): variant = 'Value' + variant
                variants.append((variant, v))
            d += '#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]\n'
            d += f'pub enum {name} {{\n'
            for idx, (variant, value) in enumerate(variants):
                if idx == 0: d += '    #[default]\n'
                d += doc(f'Wire value `{value}`.', '    ') + f'    {variant},\n'
            d += '    /// Unrecognized future wire value, retained without data loss.\n    Unknown(String),\n}\n'
            d += f'impl {name} {{\n    /// Exact wire value.\n    pub fn as_str(&self) -> &str {{ match self {{\n'
            for variant, value in variants: d += f'        Self::{variant} => {json.dumps(value)},\n'
            d += '        Self::Unknown(value) => value,\n    } }\n}\n'
            d += f'impl Serialize for {name} {{ fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {{ s.serialize_str(self.as_str()) }} }}\n'
            d += f"impl<'de> Deserialize<'de> for {name} {{ fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {{ let v = String::deserialize(d)?; Ok(match v.as_str() {{\n"
            for variant, value in variants: d += f'        {json.dumps(value)} => Self::{variant},\n'
            d += '        _ => Self::Unknown(v),\n    }) } }\n'
        elif s.get('properties'):
            secret = any('private_key' in f for f in s['properties'])
            manual_default = any(v.get('type') == 'number' and f in s.get('required', []) and not self.nullable(v) for f,v in s['properties'].items())
            derives = 'Clone, Serialize, Deserialize' + ('' if manual_default else ', Default') + ('' if secret else ', Debug')
            defaults = []
            d += f'#[derive({derives})]\npub struct {name} {{\n'
            for f, v in s['properties'].items():
                ty = self.typ(v, name + pascal(f), f)
                optional = f not in s.get('required', []) or self.nullable(v) or v.get('deprecated', False) or f == 'cursor'
                d += doc(v.get('description') or f'API field `{f}`.', '    ')
                if v.get('deprecated'): d += '    /// Deprecated upstream; retained for compatibility and optional on decode.\n'
                attrs = [f'rename = {json.dumps(f)}']
                if optional: attrs += ['default', 'skip_serializing_if = "Option::is_none"']; ty = 'Option<' + ty + '>'
                d += '    #[serde(' + ', '.join(attrs) + ')]\n'
                d += f'    pub {snake(f)}: {ty},\n'
                defaults.append(f'{snake(f)}: ' + ('serde_json::Number::from(0)' if ty == 'serde_json::Number' else 'Default::default()'))
            d += '}\n'
            if manual_default: d += f'impl Default for {name} {{ fn default() -> Self {{ Self {{ ' + ', '.join(defaults) + ' } } }\n'
            if secret: d += f'impl std::fmt::Debug for {name} {{ fn fmt(&self, f: &mut std::fmt::Formatter<\'_>) -> std::fmt::Result {{ f.write_str("{name}([REDACTED])") }} }}\n'
            if 'cursor' in s['properties'] and not name.endswith('Query'):
                optional = True
                exp = 'self.cursor.as_deref().filter(|s| !s.is_empty())' if optional else 'if self.cursor.is_empty() { None } else { Some(&self.cursor) }'
                d += f'impl {name} {{\n    /// Next cursor; empty strings and missing cursors both mean pagination is complete.\n    pub fn next_cursor(&self) -> Option<&str> {{ {exp} }}\n}}\n'
        else:
            # Prevent recursion for primitive aliases.
            d += f'pub type {name} = {self.typ(s, name)};\n'
        self.busy.remove(name)
        self.defs[name] = d

    def render(self):
        return ('// @generated by tools/generate.py; edit the generator, not this file.\n'
                '//! Typed wire models from the pinned official specification.\n'
                'use serde::{Deserialize, Serialize};\n'
                'pub use crate::{FixedPointCount, FixedPointDollars};\n\n' + '\n'.join(self.defs.values()))

def generate_rest(spec_file, out_dir, margin=False):
    s = json.loads((ROOT / spec_file).read_text(encoding='utf-8'))
    m = Models(s, margin=margin)
    for n,v in m.schemas.items():
        if n not in {'FixedPointDollars','FixedPointCount'}: m.emit(pascal(n),v)

    def resolve(x):
        if '$ref' not in x: return x
        value = s
        for part in x['$ref'][2:].split('/'): value=value[part]
        return value

    methods=[]; coverage=[]
    for path, item in s['paths'].items():
        for verb, op in item.items():
            if verb not in {'get','post','put','delete','patch'}: continue
            oid=op['operationId']; name=snake(oid.replace('Margin', '').replace('Perps', '') if margin else oid); args=[]; params=[]; pathvars=[]
            for par in item.get('parameters',[]) + op.get('parameters',[]):
                par=resolve(par)
                if par['in']=='path':
                    f=snake(par['name']);args.append(f'{f}: &str'); pathvars.append((par['name'],f))
                elif par['in']=='query':params.append(par)
                else: raise ValueError(par)
            if params:
                qname=oid+'Query'
                m.emit(qname,{'type':'object','description':f'Query parameters for `{name}`. Defaults omit optional fields; required fields must be supplied.','properties':{p['name']:{**p['schema'],'description':p.get('description','')} for p in params},'required':[p['name'] for p in params if p.get('required')]})
                args.append(f'query: &{qname}');q='Some(query)'
            else:q='None::<&()>'
            csv_fields = []
            for par in params:
                if par.get('style', 'form') != 'form':
                    raise ValueError(f'Unsupported query style in {oid}: {par}')
                if par['schema'].get('type') == 'array' and not par.get('explode', True):
                    csv_fields.append(par['name'])
            body=op.get('requestBody')
            if body:
                body=resolve(body);schema=body['content']['application/json']['schema'];btype=m.typ(schema,oid+'Body');args.append(f'body: &{btype}');b='Some(body)'
            else:b='None::<&()>'
            response=next(resolve(v) for k,v in op['responses'].items() if str(k).startswith('2'))
            schema=response.get('content',{}).get('application/json',{}).get('schema')
            ret=m.typ(schema,oid+'Response') if schema else '()'
            auth=bool(op.get('security',s.get('security')))
            d=doc(op.get('summary',oid)+'\n\n'+op.get('description','')+f'\n\n`{verb.upper()} {path}`. '+('Requires credentials.' if auth else 'Public; signed when credentials are configured.'),'    ')
            args=', '.join(['&self']+args)
            if pathvars:
                fmtpath=path
                fmtargs=[]
                for raw,f in pathvars:
                    fmtpath=fmtpath.replace('{'+raw+'}','{}');fmtargs.append('crate::client::segment('+f+')?')
                line='let path = format!('+json.dumps(fmtpath)+', '+', '.join(fmtargs)+');'
            else:line='let path = '+json.dumps(path)+';'
            patharg = '&path' if pathvars else 'path'
            receiver = 'self.client' if margin else 'self'
            request = receiver + ('.request_with_csv_query' if csv_fields else '.request')
            extra = ', &[' + ', '.join(json.dumps(f) for f in csv_fields) + ']' if csv_fields else ''
            d+=f'    pub async fn {name}({args}) -> crate::Result<{ret}> {{\n        {line}\n        {request}(reqwest::Method::{verb.upper()}, {patharg}, {q}, {b}, {str(auth).lower()}{extra}).await\n    }}\n'
            methods.append(d);coverage.append((name,verb.upper(),path,auth))
    (out_dir/'models.rs').write_text(m.render(), encoding='utf-8')
    receiver_import = "use super::{Margin, models::*};\nimpl Margin<'_> {\n" if margin else 'use crate::{Client, models::*};\nimpl Client {\n'
    (out_dir/'rest.rs').write_text('// @generated by tools/generate.py\n'+receiver_import+''.join(methods)+'}\n', encoding='utf-8')
    title = 'Margin' if margin else 'Predictions'
    retrieved = json.loads((ROOT/'spec/perps-provenance.json').read_text(encoding='utf-8'))['retrieved_on'] if margin else 'October 20, 2026'
    coverage_path = ROOT / ('docs/margin-rest-coverage.md' if margin else 'docs/rest-coverage.md')
    coverage_path.write_text('# '+title+' REST coverage\n\nGenerated from the pinned official OpenAPI '+s['info']['version']+' snapshot, retrieved '+retrieved+'. All '+str(len(coverage))+' operations in that snapshot have typed request/query/response bindings and executable HTTP implementations. This is specification coverage, not a claim of live account verification. Server-specific or undocumented endpoints are outside this table.\n\n| Method | Verb | Path | Authentication |\n| --- | --- | --- | --- |\n'+''.join(f'| `{n}` | {v} | `{p}` | '+('Required' if a else 'Optional')+' |\n' for n,v,p,a in coverage)+('\nCall these methods through `client.margin()`. Klear clearing-member APIs and FIX are separate surfaces and are not covered.\n' if margin else '\nPerps REST coverage is documented in [margin-rest-coverage.md](margin-rest-coverage.md). FIX, external feeds, and CF Benchmarks REST passthrough use separate specifications/transports and are not covered. Legacy order write paths absent from this snapshot are deliberately not exposed.\n'), encoding='utf-8', newline='\n')
    return len(coverage), len(m.defs)

def generate_websocket(spec_file, out_dir, margin=False):
    a=json.loads((ROOT/spec_file).read_text(encoding='utf-8'));w=Models(a,ws=True,margin=margin)
    for n,v in w.schemas.items():w.emit(pascal(n),v)
    (out_dir/'websocket_models.rs').write_text(w.render(), encoding='utf-8')

    # Typed dispatch for server envelopes sharing the same top-level discriminator.
    variants=[]
    for name, schema in a['components']['schemas'].items():
        tag=schema.get('properties',{}).get('type',{}).get('const')
        if not tag or name.endswith('CommandPayload') or name == 'listSubscriptionsResponsePayload': continue
        n=name.removesuffix('Payload'); n=n[0].upper()+n[1:]
        if margin: n=n.removeprefix('Margin')
        n={'SubscribedResponse':'Subscribed','UnsubscribedResponse':'Unsubscribed','OkResponse':'Ok','ErrorResponse':'Error'}.get(n,n)
        variants.append((tag,n,pascal(name)))
    d='// @generated by tools/generate.py\nuse super::models;\nuse crate::{Result, Error};\n'
    d+='/// Typed server responses. Unknown message types retain their complete JSON envelope.\n#[derive(Clone, Debug)]\npub enum Message {\n'
    for tag,n,ty in variants:d+=f'    /// Wire message `{tag}`.\n    {n}(Box<models::{ty}>),\n'
    d+='    /// Active subscriptions returned by list_subscriptions.\n    Subscriptions(Box<models::ListSubscriptionsResponsePayload>),\n    /// A future or undocumented message type.\n    Unknown(serde_json::Value),\n}\n'
    d+='''impl Message {
        /// Decode a JSON text message. Malformed known messages produce decoding errors.
        pub fn from_json(text: &str) -> Result<Self> { Self::from_slice(text.as_bytes()) }
        /// Decode a JSON byte message.
        pub fn from_slice(bytes: &[u8]) -> Result<Self> {
            let value: serde_json::Value = serde_json::from_slice(bytes)?;
            if !value.is_object() || !value.get("type").is_some_and(|v| v.is_string()) {
                return Err(Error::Config("WebSocket envelope requires a string type".into()));
            }
            if value["type"] == "ok" && value.get("msg").is_some_and(|m| m.is_array()) {
                return Ok(Self::Subscriptions(Box::new(serde_json::from_value(value)?)));
            }
    '''
    if not margin:
        d+='''        if value["type"] == "ok" && value["msg"].get("index_ids").is_some() {
                return Ok(Self::SubscribedIndicesResponse(Box::new(serde_json::from_value(value)?)));
            }
            if value["type"] == "ok" && value["msg"].get("underlying_tickers").is_some() {
                return Ok(Self::SubscribedUnderlyingsResponse(Box::new(serde_json::from_value(value)?)));
            }
            if value["type"] == "market_lifecycle_v2" && value["msg"]["event_type"] == "metadata_updated" {
                return Ok(Self::MarketMetadataUpdated(Box::new(serde_json::from_value(value)?)));
            }
    '''
    d+='        Ok(match value["type"].as_str().unwrap_or("") {\n'
    for tag,n,ty in variants:
        if n in ['SubscribedIndicesResponse','SubscribedUnderlyingsResponse','MarketMetadataUpdated']:continue
        d+=f'            "{tag}" => Self::{n}(Box::new(serde_json::from_value(value)?)),\n'
    d+='            _ => Self::Unknown(value),\n        })\n    }\n}\n'
    (out_dir/'websocket_messages.rs').write_text(d, encoding='utf-8')
    return len(w.defs)


def main():
    for margin in [False, True]:
        prefix = 'perps_' if margin else ''
        out_dir = ROOT / ('src/margin' if margin else 'src')
        out_dir.mkdir(parents=True, exist_ok=True)
        operations, models = generate_rest('spec/'+prefix+'openapi.json', out_dir, margin)
        messages = generate_websocket('spec/'+prefix+'asyncapi.json', out_dir, margin)
        print(f'Generated {"margin" if margin else "predictions"}: {operations} REST operations, {models} REST models, {messages} WebSocket models.')

if __name__ == '__main__':
    main()
