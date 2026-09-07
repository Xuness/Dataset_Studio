import {useEffect,useRef,useId} from 'react';
import type {ButtonHTMLAttributes,ReactNode} from 'react';
import {X} from 'lucide-react';
export function Button({className='',...props}:ButtonHTMLAttributes<HTMLButtonElement>){return <button {...props} className={'button '+className}/>;}
export function EmptyState({title,children,icon}:{title:string;children:ReactNode;icon?:ReactNode}){return <div className="empty-state"><div className="empty-icon">{icon}</div><h2>{title}</h2><div>{children}</div></div>;}
export function Field({label,children}:{label:string;children:ReactNode}){return <label className="field"><span>{label}</span>{children}</label>;}
export function Dialog({title,children,onClose}:{title:string;children:ReactNode;onClose:()=>void}){
  const ref=useRef<HTMLDialogElement>(null);const titleId=useId();
  useEffect(()=>{const dialog=ref.current;dialog?.showModal();return()=>dialog?.close();},[]);
  return <dialog ref={ref} className="dialog" aria-labelledby={titleId} onCancel={onClose} onClick={event=>{if(event.target===event.currentTarget)onClose();}}><div className="dialog-content"><header><h2 id={titleId}>{title}</h2><button aria-label="关闭" className="icon-button" onClick={onClose}><X size={16}/></button></header>{children}</div></dialog>;
}
