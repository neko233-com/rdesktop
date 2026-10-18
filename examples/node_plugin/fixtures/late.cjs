const wrapperArguments=arguments.length;
module.exports=()=>process.versions.node.length>0&&Buffer.from('后载').toString()==='后载'&&global===window&&typeof setImmediate==='function'&&typeof clearImmediate==='function'&&wrapperArguments===5;
