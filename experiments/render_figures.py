#!/usr/bin/env python3
"""Deterministic article figures from verified published results. Requires matplotlib."""
import json
from pathlib import Path
import matplotlib
matplotlib.use('Agg')
import matplotlib.pyplot as plt
from reproduce import ROOT, RESULTS, report


def render():
    result = report()
    out = ROOT.parent / 'news/assets'
    out.mkdir(parents=True, exist_ok=True)
    rows = json.loads((RESULTS/'deliveries.json').read_text())['rows']
    plt.rcParams.update({'svg.hashsalt':'hiroute-news-20261004','svg.fonttype':'none','font.family':'DejaVu Sans'})
    fig, ax = plt.subplots(figsize=(10, 4.8))
    for n, pair in enumerate([p for p in result['pairs'] if p['accepted']]):
        for group, offset, color, label in [('strong',-.18,'#5564bd','All Astra'),('mixed',.18,'#078677','HiRoute mixed')]:
            row=next(r for r in rows if r['id']==f"formal-draft98-v1-p{pair['pair']:02d}-{group}")
            lo,hi=map(float,(row['usd']['lower'],row['usd']['upper']))
            ax.barh(n+offset,(lo+hi)/2,height=.27,color=color,label=label if n==0 else None)
            ax.errorbar((lo+hi)/2,n+offset,xerr=[[(hi-lo)/2],[(hi-lo)/2]],fmt='none',color='#20283e',capsize=3)
            ax.text(hi+.025,n+offset,f'${lo:.3f}–{hi:.3f}',va='center',fontsize=11)
        ax.text(2.46,n, pair['conservative_savings_percent']+'% less',va='center',ha='right',fontsize=12,fontweight='bold',color='#087464')
    ax.set_yticks([0,1],['Pair 2','Pair 3']);ax.invert_yaxis();ax.set_xlim(0,2.5)
    ax.set_xlabel('Complete-task API-equivalent cost (USD)');ax.spines[['top','right']].set_visible(False)
    ax.legend(frameon=False,loc='lower right');ax.set_title('Same acceptance gate. Over 90% lower cost.',loc='left',pad=20,fontsize=19)
    fig.text(.08,.025,'Two of three primary pairs passed the original whole-delivery gate. All outcomes are in the experiment record.\nBounds include subject attempts and routing evaluation; savings compare mixed upper against Astra lower. Not subscription invoices.',fontsize=9,color='#566078')
    fig.tight_layout(rect=[0,.13,1,1])
    for lang in ['zh','en']:
        target=out/f'research-cost-{lang}.svg'
        fig.savefig(target,metadata={'Date':None})
        target.write_text('\n'.join(line.rstrip() for line in target.read_text().splitlines())+'\n')
    plt.close(fig)
    for lang in ['zh','en']:
        labels=(['长任务自动模型接力','Qwen 执行','上下文交接 · 重新评估','Astra 接续','25 次模型请求','0.485 < 0.5 胜任度下限','13 次实际工具调用','31 分 36 秒 · 零中途人工提示 · 343 项独立验收通过','单个 HTTPX 任务的实际记录；此案例不主张成本节省。'] if lang=='zh' else ['Automatic model handoff','Qwen executes','Context handoff · Reassess','Astra continues','25 model requests','Competence 0.485 < 0.5 floor','13 actual tool calls','31m 36s · No intermediate operator prompts · 343 passing assertions','Recorded HTTPX case; no cost-savings claim for this experiment.'])
        from html import escape
        svg=['<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1100 320" role="img"><title>'+escape(labels[0])+'</title><rect width="1100" height="320" rx="20" fill="#eef2fa"/><g font-family="system-ui,sans-serif" fill="#192439">',f'<text x="40" y="48" font-size="26" font-weight="700">{escape(labels[0])}</text>']
        for i,x in enumerate([40,410,780]):
            svg.append(f'<rect x="{x}" y="85" width="280" height="110" rx="14" fill="white"/><text x="{x+20}" y="123" font-size="20" font-weight="700">{escape(labels[i+1])}</text><text x="{x+20}" y="163" font-size="16">{escape(labels[i+4])}</text>')
            if i<2:svg.append(f'<path d="M {x+290} 140 h 65 m -10 -7 l 10 7 -10 7" fill="none" stroke="#586caa" stroke-width="3"/>')
        svg.extend([f'<text x="40" y="246" font-size="22" font-weight="700">{escape(labels[7])}</text>',f'<text x="40" y="287" font-size="15" fill="#536079">{escape(labels[8])}</text>','</g></svg>'])
        (out/f'unattended-handoff-{lang}.svg').write_text(''.join(svg)+'\n')


if __name__=='__main__':render()
