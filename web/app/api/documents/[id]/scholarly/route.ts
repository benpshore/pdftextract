import {owner,ownedRecord,failure,boundedBody} from '@/lib/server';
import {attachScholarly,scholarlyStatus} from '@/lib/scholarly-service';
type Context={params:Promise<{id:string}>};
export async function GET(request:Request,context:Context){try{
  const user=await owner(request),{id}=await context.params,record=await ownedRecord(id,user);
  return Response.json(await scholarlyStatus(record,request.signal),{headers:{'Cache-Control':'private, no-store'}});
}catch(error){return failure(error);}}
export async function POST(request:Request,context:Context){try{
  const user=await owner(request),{id}=await context.params,record=await ownedRecord(id,user);
  const input=JSON.parse(new TextDecoder().decode(await boundedBody(request,4096)));
  if(!input||typeof input!=='object'||Array.isArray(input)||Object.keys(input).length!==1||!Object.hasOwn(input,'baseResultKey')||(input.baseResultKey!==null&&typeof input.baseResultKey!=='string'))throw new Response('A current result revision is required.',{status:400});
  return Response.json(await attachScholarly(record,user,input.baseResultKey,request.signal),{headers:{'Cache-Control':'private, no-store'}});
}catch(error){return failure(error);}}
