local H={}
local function color(hex,a) return {tonumber(hex:sub(2,3),16)/255,tonumber(hex:sub(4,5),16)/255,tonumber(hex:sub(6,7),16)/255,a or 1} end
local gold=color('#ceb88a');local pale=color('#d5dad5');local muted=color('#8e9ca3')
local function text(ui,anchor,x,y,t,size,c) ui:screen_label(anchor,x,y,t,{size=size,color=c or pale,backdrop=true}) end
function H.draw(ui,g,w)
 local compact=ui:get_viewport_size().width<1050;local room=w.rooms[g.room]
 text(ui,'top_left',24,20,'THE HOLLOW CROWN',compact and 22 or 28,gold)
 text(ui,'top_left',24,56,'EXPEDITION '..g.expeditions..'   /   WON '..g.victories..'   /   LOST '..g.defeats,11,muted)
 text(ui,'top_center',0,24,room.name,compact and 19 or 25)
 local minutes=math.floor(g.elapsed/60);local seconds=math.floor(g.elapsed%60)
 text(ui,'top_center',0,57,'STOP '..g.stop..' OF 14    '..string.format('%02d:%02d',minutes,seconds)..'    '..(g.pressure>1.12 and 'DIRE OMENS' or g.pressure<.95 and 'A QUIETER DARK' or 'GRIM OMENS'),11,gold)
 text(ui,'top_right',24,22,g.auto and 'SPECTATOR' or 'YOUR COMPANY',15,gold)
 text(ui,'top_right',24,48,g.gold..' gold  /  '..g.potions..' draughts  /  '..g.kills..' slain',12)
 if g.lastRoll then text(ui,'top_right',24,73,'d20 '..g.lastRoll.die..' + '..g.lastRoll.bonus..'   '..g.lastRoll.result,12,g.lastRoll.die==20 and gold or muted) end
 local overlay=g.inventory or g.map
 local step=compact and 190 or 250
 if not overlay then
 for i,h in ipairs(g.heroes) do
  local x=24+(i-1)*step;local c=color(({'#92bdd0','#b2bc81','#b4a1d5'})[i])
  text(ui,'bottom_left',x,136,string.upper(h.name),15,c)
  text(ui,'bottom_left',x,113,h.gear.weapon.name,11,color(h.gear.weapon.color))
  text(ui,'bottom_left',x,93,h.gear.armor.name..'  /  AC '..h.ac,10,muted)
  local filled=math.max(0,math.min(14,math.ceil(h.hp/h.maxhp*14)))
  text(ui,'bottom_left',x,70,string.rep('|',filled)..string.rep('.',14-filled),15,h.hp>0 and c or color('#9c6660'))
  text(ui,'bottom_left',x,48,h.hp<=0 and 'FALLEN - SHRINE REQUIRED' or math.floor(h.hp)..' / '..h.maxhp..' HP'..(h.class=='mage' and '   '..math.floor(h.mana)..' mana' or '')..(h.poison>0 and '  POISON' or ''),11)
 end
 local objective=({intro=room.subtitle,fight='Defeat the guardians. Protect the wounded.',loot=g.lootGiven and 'The company examines its new equipment.' or 'Search the reliquary.',travel='Following the passage to '..w.rooms[g.nextRoom or g.room].name,branch='Choose the next passage.',victory='The survivors claim the Crown.',defeat='The expedition is lost. A new company will descend.'})[g.phase]
 text(ui,'bottom_center',0,190,objective,13,gold)
 text(ui,'bottom_center',0,166,g.upgrades..' upgrades   /   '..g.lootCount..' items found   /   '..g.hazardHits..' hazard wounds',11,muted)
 if not compact then
  text(ui,'bottom_right',24,166,'CHRONICLE',11,gold)
  for i,m in ipairs(g.journal) do text(ui,'bottom_right',24,166-i*21,m.text,11,color(m.color,math.max(.35,1-(i-1)*.11))) end
 end
 text(ui,'bottom_center',0,16,g.auto and 'TAB command  /  P pause  /  1-3 speed  /  I gear  /  V atlas  /  M sound  /  R restart' or 'WASD / click move  /  SPACE strike  /  Q nova  /  E draught  /  F search  /  I gear  /  TAB watch',11,muted)
 end
 if g.result and not overlay then
  text(ui,'center',0,0,g.result=='victory' and 'THE CROWN HAS A NEW MASTER' or 'THE ABBEY TAKES THEIR NAMES',compact and 27 or 36,gold)
  text(ui,'center',0,48,'Reached '..room.name..'  /  '..minutes..' minutes  /  '..g.upgrades..' upgrades',14)
 elseif g.paused and not overlay then text(ui,'center',0,0,'PAUSED',36,gold)
 elseif g.banner>0 and not overlay then text(ui,'center',0,105,room.subtitle,15,color('#c9bfa8',math.min(1,g.banner))) end
 if not overlay then
 for _,e in ipairs(g.enemies) do if e.active and e.hp>0 then
  if e.boss then ui:world_label(e.x,4.8,e.z,string.upper(e.name)..'  '..e.hp..' / '..e.maxhp,{size=14,color=gold,backdrop=true})
  elseif e.hp<e.maxhp then ui:world_label(e.x,e.kind=='spider' and 1.8 or 2.6,e.z,tostring(e.hp),{size=11,color=color('#c9a58a'),backdrop=true}) end
 end end
 for _,f in ipairs(g.floats) do ui:world_label(f.x,f.y,f.z,f.text,{size=17,color=color(f.color,math.min(1,f.life))}) end
 for _,t in ipairs(g.hazards) do ui:world_label(t.x,.8,t.z,string.upper(t.kind)..'!',{size=12,color=color('#e3ae79'),backdrop=true}) end
 if g.phase=='loot' and not g.lootGiven then ui:world_label(room.x+room.loot[1],1.9,room.z+room.loot[2],'RELIQUARY',{size=12,color=gold,backdrop=true}) end
 end
 ui:anchored_panel('watch_controls','top_right',24,102,185,nil,function(p)
  p:horizontal(function(row)
   if row:button(g.auto and 'Take command' or 'Watch','mode') then g.auto=not g.auto;g.target=nil end
   if row:button(g.paused and 'Resume' or 'Pause','pause') then g.paused=not g.paused end
  end)
  p:horizontal(function(row) for _,s in ipairs({1,2,4}) do if row:button((g.speed==s and '* ' or '')..s..'x','speed'..s) then g.speed=s end end end)
  p:horizontal(function(row) if row:button('Equipment','gear') then g.inventory=not g.inventory end;if row:button('Atlas','map') then g.map=not g.map end end)
 end)
 if g.phase=='branch' then
  ui:anchored_panel('branch','center',0,0,420,nil,function(p)
   p:heading('Two ways deeper',{color=gold,size=22})
   for i,n in ipairs(g.branchChoices) do
    local r=w.rooms[n];p:label(r.subtitle,{color=muted,size=13})
    if p:button((i==1 and 'J  ' or 'K  ')..r.name,'branch'..n) then g.selectedBranch=n end
   end
   if g.auto then p:label('The company considers its wounds and supplies.',{color=gold,size=12}) end
  end)
 end
 if g.inventory then
  ui:anchored_panel('inventory','center',0,0,560,500,function(p)
   p:heading('Equipment of the company',{color=gold,size=24})
   for _,h in ipairs(g.heroes) do
    p:separator();p:heading(h.name..(h.hp<=0 and ' - FALLEN' or ' - '..math.floor(h.hp)..' / '..h.maxhp..' HP'),{color=pale,size=17})
    for _,slot in ipairs({'weapon','armor','boots','charm'}) do local item=h.gear[slot];p:label(string.upper(slot)..'   '..item.name..'   '..(item.rarity or 'Starting gear'),{color=color(item.color),size=13}) end
    p:label('Attack + '..h.hit..' / bonus damage '..h.damage..' / armor '..h.ac..' / resistance '..math.floor(h.resist*100)..'%',{color=muted,size=12})
   end
   p:label('Better finds are equipped automatically; lesser items are salvaged.',{color=gold,size=12})
   if p:button('Close equipment','closegear') then g.inventory=false end
  end)
 end
 if g.map then
  ui:anchored_panel('atlas','top_left',24,95,245,nil,function(p)
   p:heading('The abbey below',{color=gold,size=20})
   for i,r in ipairs(w.rooms) do p:label((i==g.room and '> ' or g.visited[i] and '+ ' or '. ')..i..'  '..r.name,{color=i==g.room and gold or g.visited[i] and pale or muted,size=12}) end
   p:separator();p:label('Forks: Mercy / Silk; Refuge / Foundry',{color=gold,size=11})
   if p:button('Close atlas','closemap') then g.map=false end
  end)
 end
end
return H
