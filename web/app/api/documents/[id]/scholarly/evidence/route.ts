import {owner,ownedRecord,failure} from '@/lib/server';
import {scholarlyEvidence} from '@/lib/scholarly-service';
export async function GET(request:Request,context:{params:Promise<{id:string}>}){try{
  const user=await owner(request),{id}=await context.params,record=await ownedRecord(id,user);
  return await scholarlyEvidence(record,new URL(request.url).searchParams.get('format')||'native-json');
}catch(error){return failure(error);}}
